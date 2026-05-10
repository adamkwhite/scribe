use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use scribe_core::notes::NotesGenerator;
use scribe_core::{audio, config, logging, notes, opener, transcribe};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthStr;

mod playback;
mod session_store;
mod utils;

use utils::sessions;

const ACTION_PANEL_WIDTH: u16 = 32;
const FOOTER_HEIGHT: u16 = 3;

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Screen {
    Setup,
    Sessions,
    SessionDetail,
    TextViewer,
    Playback,
    EditSessionName,
    NewRecording,
    Recording,
    Processing,
    Complete,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupFocus {
    ApiKey,
    NotesModel,
    WhisperBin,
    WhisperModel,
    OutputDir,
    Validate,
    Download,
    Save,
    Quit,
}

#[derive(Clone, Debug)]
struct SetupForm {
    openrouter_api_key: String,
    model: String,
    whisper_bin: String,
    whisper_model: String,
    output_dir: String,
    focus: SetupFocus,
    message: String,
}

struct RecordingState {
    session_dir: PathBuf,
    session_name: String,
    started_at: Instant,
    recording_flag: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<Result<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessingStep {
    Finalizing,
    Transcribing,
    GeneratingNotes,
    WritingNotes,
    Complete,
}

#[derive(Debug)]
enum ProcessingEvent {
    Step(ProcessingStep),
    RecordingStatus(String),
    Complete,
    Error(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DetailAction {
    Notes,
    Transcript,
    Playback,
    OpenFolder,
    Rename,
    Archive,
    Delete,
}

const DETAIL_ACTIONS: [DetailAction; 7] = [
    DetailAction::Notes,
    DetailAction::Transcript,
    DetailAction::Playback,
    DetailAction::OpenFolder,
    DetailAction::Rename,
    DetailAction::Archive,
    DetailAction::Delete,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingSessionAction {
    Archive,
    Delete,
}

struct TextViewerState {
    title: String,
    path: PathBuf,
    lines: Vec<String>,
    scroll: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorkScreenLayout {
    main: Rect,
    actions: Rect,
    footer: Rect,
}

impl TextViewerState {
    fn new(title: impl Into<String>, path: PathBuf, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            path,
            lines,
            scroll: 0,
        }
    }

    fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    fn scroll_down(&mut self, amount: usize) {
        self.scroll = (self.scroll + amount).min(self.max_scroll());
    }

    fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }
}

struct App {
    screen: Screen,
    setup_return_screen: Option<Screen>,
    cfg: Option<config::Config>,
    log_path: Option<PathBuf>,
    setup: SetupForm,
    sessions: Vec<sessions::SessionEntry>,
    selected_session: usize,
    recording_name: String,
    recording: Option<RecordingState>,
    recording_messages: Vec<String>,
    processing_session: Option<PathBuf>,
    processing_step: ProcessingStep,
    processing_rx: Option<mpsc::UnboundedReceiver<ProcessingEvent>>,
    message: String,
    selected_detail_action: usize,
    detail_session: Option<sessions::SessionEntry>,
    text_viewer: Option<TextViewerState>,
    playback: Option<playback::PlaybackViewState>,
    pending_session_action: Option<PendingSessionAction>,
}

pub async fn run() -> Result<()> {
    let log_path = logging::init_file_logging("scribe-tui")?;
    tracing::info!(log_path = %log_path.display(), "scribe TUI starting");
    let mut app = App::load(Some(log_path))?;
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &mut app).await;
    restore_terminal(&mut terminal)?;
    if let Err(error) = &result {
        tracing::error!(error = %error, "scribe TUI exited with error");
    } else {
        tracing::info!("scribe TUI exiting");
    }
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("Failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;
    Terminal::new(CrosstermBackend::new(stdout)).context("Failed to initialize terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("Failed to disable terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("Failed to leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")
}

async fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        app.drain_processing_events();
        terminal.draw(|frame| render(frame, app))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && handle_key(app, key).await?
        {
            return Ok(());
        }
    }
}

async fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match app.screen {
        Screen::Setup => handle_setup_key(app, key).await,
        Screen::Sessions => handle_sessions_key(app, key),
        Screen::SessionDetail => handle_session_detail_key(app, key),
        Screen::TextViewer => handle_text_viewer_key(app, key),
        Screen::Playback => handle_playback_key(app, key),
        Screen::EditSessionName => handle_edit_session_name_key(app, key),
        Screen::NewRecording => handle_new_recording_key(app, key).await,
        Screen::Recording => handle_recording_key(app, key).await,
        Screen::Processing => Ok(false),
        Screen::Complete => handle_complete_key(app, key),
        Screen::Error => handle_error_key(app, key),
    }
}

async fn handle_setup_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => return Ok(app.close_setup()),
        KeyCode::Tab | KeyCode::Down => app.setup.focus = next_setup_focus(app.setup.focus),
        KeyCode::BackTab | KeyCode::Up => app.setup.focus = previous_setup_focus(app.setup.focus),
        KeyCode::Backspace => app.setup.delete_char(),
        KeyCode::Char(ch) => app.setup.push_char(ch),
        KeyCode::Enter => match app.setup.focus {
            SetupFocus::Validate => app.validate_setup(),
            SetupFocus::Download => app.download_model().await,
            SetupFocus::Save => {
                if let Err(error) = app.save_setup() {
                    app.setup.message = error.to_string();
                }
            }
            SetupFocus::Quit => return Ok(true),
            _ => app.setup.focus = next_setup_focus(app.setup.focus),
        },
        _ => {}
    }
    Ok(false)
}

fn handle_sessions_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Up => app.select_previous_session(),
        KeyCode::Down => app.select_next_session(),
        KeyCode::Enter | KeyCode::Char('o') => app.open_selected_session_detail(),
        KeyCode::Char('r') => {
            app.recording_name.clear();
            app.message.clear();
            app.screen = Screen::NewRecording;
        }
        KeyCode::Char('s') => {
            app.setup = SetupForm::from_config(app.cfg.as_ref());
            app.setup_return_screen = Some(app.screen.clone());
            app.screen = Screen::Setup;
        }
        KeyCode::Char('f') => app.reload_sessions(),
        _ => {}
    }
    Ok(false)
}

fn handle_session_detail_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if app.pending_session_action.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => app.confirm_pending_session_action()?,
            KeyCode::Char('n') | KeyCode::Esc => app.cancel_pending_session_action(),
            KeyCode::Char('q') => return Ok(true),
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('b') => {
            app.stop_playback();
            app.screen = Screen::Sessions;
        }
        KeyCode::Up => app.select_previous_detail_action(),
        KeyCode::Down => app.select_next_detail_action(),
        KeyCode::Enter => app.activate_detail_action()?,
        KeyCode::Char('o') => app.open_detail_session_folder()?,
        KeyCode::Char('n') => app.open_text_artifact("notes.md", "Notes")?,
        KeyCode::Char('t') => app.open_text_artifact("transcript.txt", "Transcript")?,
        KeyCode::Char('p') => app.open_playback_view(),
        KeyCode::Char('e') => app.open_edit_session_name(),
        KeyCode::Char('a') => app.request_session_action(PendingSessionAction::Archive),
        KeyCode::Char('d') => app.request_session_action(PendingSessionAction::Delete),
        KeyCode::Char('q') => return Ok(true),
        _ => {}
    }
    Ok(false)
}

fn handle_text_viewer_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if let Some(viewer) = &mut app.text_viewer {
        match key.code {
            KeyCode::Esc | KeyCode::Char('b') => app.screen = Screen::SessionDetail,
            KeyCode::Up | KeyCode::Char('k') => viewer.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => viewer.scroll_down(1),
            KeyCode::PageUp => viewer.scroll_up(10),
            KeyCode::PageDown => viewer.scroll_down(10),
            KeyCode::Home | KeyCode::Char('g') => viewer.scroll_to_top(),
            KeyCode::End | KeyCode::Char('G') => viewer.scroll_to_bottom(),
            KeyCode::Char('q') => return Ok(true),
            _ => {}
        }
    } else {
        app.screen = Screen::SessionDetail;
    }
    Ok(false)
}

fn handle_playback_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('b') => {
            app.stop_playback();
            app.screen = Screen::SessionDetail;
        }
        KeyCode::Char(' ') => app.toggle_playback(),
        KeyCode::Char('r') => app.restart_playback(),
        KeyCode::Left | KeyCode::Char('h') => app.rewind_playback(),
        KeyCode::Right | KeyCode::Char('l') => app.fast_forward_playback(),
        KeyCode::Char('s') => app.stop_playback(),
        KeyCode::Char('q') => return Ok(true),
        _ => {}
    }
    Ok(false)
}

fn handle_edit_session_name_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.message.clear();
            app.screen = Screen::SessionDetail;
        }
        KeyCode::Backspace => {
            app.recording_name.pop();
            app.message.clear();
        }
        KeyCode::Char(ch) => {
            app.recording_name.push(ch);
            app.message.clear();
        }
        KeyCode::Enter => app.rename_detail_session()?,
        _ => {}
    }
    Ok(false)
}

async fn handle_new_recording_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.screen = Screen::Sessions,
        KeyCode::Backspace => {
            app.recording_name.pop();
            app.message.clear();
        }
        KeyCode::Char(ch) => {
            app.recording_name.push(ch);
            app.message.clear();
        }
        KeyCode::Enter => app.start_recording().await?,
        _ => {}
    }
    Ok(false)
}

async fn handle_recording_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if matches!(key.code, KeyCode::Enter | KeyCode::Char('s')) {
        app.stop_and_process().await?;
    }
    Ok(false)
}

fn handle_complete_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Enter | KeyCode::Char('v') => app.open_processing_session()?,
        KeyCode::Esc | KeyCode::Char('b') => {
            app.reload_sessions();
            app.screen = Screen::Sessions;
        }
        KeyCode::Char('q') => return Ok(true),
        _ => {}
    }
    Ok(false)
}

fn handle_error_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('b') => {
            app.reload_sessions();
            app.screen = Screen::Sessions;
        }
        KeyCode::Char('q') => return Ok(true),
        _ => {}
    }
    Ok(false)
}

impl App {
    fn load(log_path: Option<PathBuf>) -> Result<Self> {
        cleanup_archive_on_startup();
        let cfg = utils::config::load_existing_config()?;
        let setup = SetupForm::from_config(cfg.as_ref());
        let mut app = Self {
            screen: Screen::Setup,
            setup_return_screen: None,
            cfg,
            log_path,
            setup,
            sessions: Vec::new(),
            selected_session: 0,
            recording_name: String::new(),
            recording: None,
            recording_messages: Vec::new(),
            processing_session: None,
            processing_step: ProcessingStep::Finalizing,
            processing_rx: None,
            message: String::new(),
            selected_detail_action: 0,
            detail_session: None,
            text_viewer: None,
            playback: None,
            pending_session_action: None,
        };

        if app
            .cfg
            .as_ref()
            .is_some_and(|cfg| config::validate_setup(cfg).is_ok())
        {
            app.reload_sessions();
            app.screen = Screen::Sessions;
        }

        Ok(app)
    }

    fn validate_setup(&mut self) {
        match self.setup.to_config().and_then(|cfg| {
            let config_dir = config::config_dir()?;
            let cfg = config::resolve_managed_whisper_model_config(cfg, &config_dir);
            config::validate_setup(&cfg)?;
            Ok(cfg)
        }) {
            Ok(_) => {
                tracing::info!("TUI setup validation succeeded");
                self.setup.message = "Setup looks valid.".to_string();
            }
            Err(error) => {
                tracing::warn!(error = %error, "TUI setup validation failed");
                self.setup.message = error.to_string();
            }
        }
    }

    async fn download_model(&mut self) {
        self.setup.message = "Downloading model...".to_string();
        let mut last_message = self.setup.message.clone();
        match config::ensure_managed_whisper_model_with_events(|event| {
            last_message = event.message();
            tracing::info!(message = %last_message, "TUI model download status");
        })
        .await
        {
            Ok(path) => {
                self.setup.whisper_model = path.to_string_lossy().into_owned();
                self.setup.message = last_message;
            }
            Err(error) => {
                tracing::warn!(error = %error, "TUI model download failed");
                self.setup.message = error.to_string();
            }
        }
    }

    fn save_setup(&mut self) -> Result<()> {
        let config_dir = config::config_dir()?;
        let cfg =
            config::resolve_managed_whisper_model_config(self.setup.to_config()?, &config_dir);
        config::validate_setup(&cfg)?;
        utils::config::save_config(&cfg)?;
        tracing::info!("TUI setup saved");
        self.cfg = Some(cfg);
        self.reload_sessions();
        self.screen = self.setup_return_screen.take().unwrap_or(Screen::Sessions);
        Ok(())
    }

    fn close_setup(&mut self) -> bool {
        if let Some(screen) = self.setup_return_screen.take() {
            self.screen = screen;
            false
        } else {
            true
        }
    }

    fn reload_sessions(&mut self) {
        if let Some(cfg) = &self.cfg {
            match config::effective_output_dir(cfg).and_then(|dir| sessions::list_sessions(&dir)) {
                Ok(sessions) => {
                    self.sessions = sessions;
                    if self.selected_session >= self.sessions.len() {
                        self.selected_session = self.sessions.len().saturating_sub(1);
                    }
                    self.message.clear();
                }
                Err(error) => {
                    tracing::warn!(error = %error, "TUI session reload failed");
                    self.message = error.to_string();
                }
            }
        }
    }

    fn select_previous_session(&mut self) {
        self.selected_session = self.selected_session.saturating_sub(1);
    }

    fn select_next_session(&mut self) {
        if self.selected_session + 1 < self.sessions.len() {
            self.selected_session += 1;
        }
    }

    fn open_selected_session_detail(&mut self) {
        if let Some(session) = self.sessions.get(self.selected_session) {
            tracing::info!(session_dir = %session.path.display(), "TUI opening session detail");
            self.detail_session = Some(session.clone());
            self.selected_detail_action = 0;
            self.message.clear();
            self.screen = Screen::SessionDetail;
        }
    }

    fn select_previous_detail_action(&mut self) {
        self.selected_detail_action = self.selected_detail_action.saturating_sub(1);
    }

    fn select_next_detail_action(&mut self) {
        if self.selected_detail_action + 1 < DETAIL_ACTIONS.len() {
            self.selected_detail_action += 1;
        }
    }

    fn activate_detail_action(&mut self) -> Result<()> {
        match DETAIL_ACTIONS[self.selected_detail_action] {
            DetailAction::Notes => self.open_text_artifact("notes.md", "Notes")?,
            DetailAction::Transcript => self.open_text_artifact("transcript.txt", "Transcript")?,
            DetailAction::Playback => self.open_playback_view(),
            DetailAction::OpenFolder => self.open_detail_session_folder()?,
            DetailAction::Rename => self.open_edit_session_name(),
            DetailAction::Archive => self.request_session_action(PendingSessionAction::Archive),
            DetailAction::Delete => self.request_session_action(PendingSessionAction::Delete),
        }
        Ok(())
    }

    fn detail_session(&self) -> Option<&sessions::SessionEntry> {
        self.detail_session.as_ref()
    }

    fn open_detail_session_folder(&mut self) -> Result<()> {
        if let Some(session) = self.detail_session() {
            tracing::info!(session_dir = %session.path.display(), "TUI opening session folder");
            opener::open_folder(&session.path)?;
        }
        Ok(())
    }

    fn open_text_artifact(&mut self, filename: &str, title: &str) -> Result<()> {
        let Some(session) = self.detail_session() else {
            return Ok(());
        };
        let path = session.path.join(filename);
        if !path.exists() {
            self.message = format!("{filename} is not available for this session.");
            return Ok(());
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let lines = contents
            .lines()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        self.text_viewer = Some(TextViewerState::new(title, path, lines));
        self.message.clear();
        self.screen = Screen::TextViewer;
        Ok(())
    }

    fn open_playback_view(&mut self) {
        let Some(session) = self.detail_session() else {
            return;
        };
        let path = session.path.join("recording.wav");
        if !path.exists() {
            self.message = "recording.wav is not available for this session.".to_string();
            return;
        }

        let session_name = session.name.clone();
        self.stop_playback();
        self.playback = Some(playback::PlaybackViewState::open(
            session_name,
            path.clone(),
        ));
        tracing::info!(recording_path = %path.display(), "TUI opening playback view");
        self.message.clear();
        self.screen = Screen::Playback;
    }

    fn open_edit_session_name(&mut self) {
        let Some(session) = self.detail_session() else {
            return;
        };
        self.recording_name = editable_session_name(&session.name);
        self.pending_session_action = None;
        self.message.clear();
        self.screen = Screen::EditSessionName;
    }

    fn rename_detail_session(&mut self) -> Result<()> {
        let name = match session_store::validate_session_name(&self.recording_name) {
            Ok(name) => name.to_string(),
            Err(error) => {
                self.message = error.to_string();
                return Ok(());
            }
        };
        let Some(session) = self.detail_session.clone() else {
            return Ok(());
        };

        self.stop_playback();
        let renamed_path = session_store::rename_session(&session.path, &name)?;
        let new_name = renamed_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(name);
        let modified = std::fs::metadata(&renamed_path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(session.modified);
        tracing::info!(
            old_session_dir = %session.path.display(),
            new_session_dir = %renamed_path.display(),
            "TUI renamed session"
        );
        self.detail_session = Some(sessions::SessionEntry {
            path: renamed_path,
            name: new_name.clone(),
            status: session.status,
            modified,
            recorded_at: sessions::recorded_at_from_session_name(&new_name),
        });
        self.reload_sessions();
        self.message = "Session renamed.".to_string();
        self.screen = Screen::SessionDetail;
        Ok(())
    }

    fn toggle_playback(&mut self) {
        if let Some(playback) = &mut self.playback
            && let Some(controller) = playback.controller.as_mut()
        {
            match controller.toggle_play_pause() {
                Ok(()) => {
                    playback.status = if controller.is_paused() {
                        "Paused".to_string()
                    } else {
                        "Playing".to_string()
                    };
                }
                Err(error) => playback.error = Some(error.to_string()),
            }
        }
    }

    fn restart_playback(&mut self) {
        if let Some(playback) = &mut self.playback
            && let Some(controller) = playback.controller.as_mut()
        {
            match controller.restart() {
                Ok(()) => playback.status = "Playing from start".to_string(),
                Err(error) => playback.error = Some(error.to_string()),
            }
        }
    }

    fn rewind_playback(&mut self) {
        if let Some(playback) = &mut self.playback
            && let Some(controller) = playback.controller.as_mut()
        {
            match controller.rewind(Duration::from_secs(10)) {
                Ok(()) => playback.status = "Rewound 10 seconds".to_string(),
                Err(error) => playback.error = Some(error.to_string()),
            }
        }
    }

    fn fast_forward_playback(&mut self) {
        if let Some(playback) = &mut self.playback
            && let Some(controller) = playback.controller.as_mut()
        {
            match controller.fast_forward(Duration::from_secs(30)) {
                Ok(()) => playback.status = "Fast-forwarded 30 seconds".to_string(),
                Err(error) => playback.error = Some(error.to_string()),
            }
        }
    }

    fn stop_playback(&mut self) {
        if let Some(playback) = &mut self.playback {
            playback.stop();
        }
    }

    fn request_session_action(&mut self, action: PendingSessionAction) {
        self.pending_session_action = Some(action);
        self.message = match action {
            PendingSessionAction::Archive => {
                "Archive this session? Press y to confirm or n to cancel.".to_string()
            }
            PendingSessionAction::Delete => {
                "Delete this session permanently? Press y to confirm or n to cancel.".to_string()
            }
        };
    }

    fn cancel_pending_session_action(&mut self) {
        self.pending_session_action = None;
        self.message = "Cancelled.".to_string();
    }

    fn confirm_pending_session_action(&mut self) -> Result<()> {
        let Some(action) = self.pending_session_action.take() else {
            return Ok(());
        };
        match action {
            PendingSessionAction::Archive => self.archive_detail_session(),
            PendingSessionAction::Delete => self.delete_detail_session(),
        }
    }

    fn archive_detail_session(&mut self) -> Result<()> {
        let config_dir = config::config_dir()?;
        let archive_root = session_store::archive_dir(&config_dir);
        self.archive_detail_session_to(&archive_root)
    }

    fn archive_detail_session_to(&mut self, archive_root: &std::path::Path) -> Result<()> {
        let Some(session) = self.detail_session.clone() else {
            return Ok(());
        };
        self.stop_playback();
        let archived_path = session_store::archive_session(&session.path, archive_root)?;
        tracing::info!(
            session_dir = %session.path.display(),
            archive_dir = %archived_path.display(),
            "TUI archived session"
        );
        self.detail_session = None;
        self.text_viewer = None;
        self.playback = None;
        self.reload_sessions();
        self.message = format!("Archived session to {}", archived_path.display());
        self.screen = Screen::Sessions;
        Ok(())
    }

    fn delete_detail_session(&mut self) -> Result<()> {
        let Some(session) = self.detail_session.clone() else {
            return Ok(());
        };
        self.stop_playback();
        session_store::delete_session(&session.path)?;
        tracing::info!(session_dir = %session.path.display(), "TUI deleted session");
        self.detail_session = None;
        self.text_viewer = None;
        self.playback = None;
        self.reload_sessions();
        self.message = format!("Deleted session {}", session.name);
        self.screen = Screen::Sessions;
        Ok(())
    }

    fn open_processing_session(&mut self) -> Result<()> {
        if let Some(path) = &self.processing_session {
            tracing::info!(session_dir = %path.display(), "TUI opening processed session folder");
            opener::open_folder(path)?;
        }
        Ok(())
    }

    async fn start_recording(&mut self) -> Result<()> {
        let name = match session_store::validate_session_name(&self.recording_name) {
            Ok(name) => name.to_string(),
            Err(error) => {
                self.message = error.to_string();
                return Ok(());
            }
        };
        let cfg = self.cfg.as_ref().context("Scribe is not configured")?;
        let session_dir = audio::create_session_dir(cfg, Some(&name))?;
        tracing::info!(session_dir = %session_dir.display(), "TUI recording session created");
        let recording = Arc::new(AtomicBool::new(true));
        let recording_for_task = recording.clone();
        let sample_rate = cfg.sample_rate;
        let session_for_task = session_dir.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        let tx_for_task = tx.clone();

        let task = tokio::task::spawn_blocking(move || {
            audio::record_loopback_with_events(
                recording_for_task,
                sample_rate,
                session_for_task,
                move |event| {
                    let _ = tx_for_task.send(ProcessingEvent::RecordingStatus(event.message()));
                },
            )
        });

        self.recording_messages.clear();
        self.processing_rx = Some(rx);
        self.recording = Some(RecordingState {
            session_name: session_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Recording".to_string()),
            session_dir,
            started_at: Instant::now(),
            recording_flag: recording,
            task,
        });
        self.processing_step = ProcessingStep::Finalizing;
        self.message.clear();
        self.screen = Screen::Recording;
        tracing::info!("TUI recording started");
        Ok(())
    }

    async fn stop_and_process(&mut self) -> Result<()> {
        let Some(recording) = self.recording.take() else {
            return Ok(());
        };
        let Some(cfg) = self.cfg.clone() else {
            anyhow::bail!("Scribe is not configured");
        };

        self.processing_session = Some(recording.session_dir.clone());
        self.processing_step = ProcessingStep::Finalizing;
        self.screen = Screen::Processing;
        recording.recording_flag.store(false, Ordering::Relaxed);
        tracing::info!(
            session_dir = %recording.session_dir.display(),
            "TUI recording stop requested; processing starting"
        );
        let (tx, rx) = mpsc::unbounded_channel();
        self.processing_rx = Some(rx);
        spawn_processing_task(tx, cfg, recording);
        Ok(())
    }

    fn drain_processing_events(&mut self) {
        let mut events = Vec::new();
        if let Some(rx) = &mut self.processing_rx {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            match event {
                ProcessingEvent::Step(step) => self.processing_step = step,
                ProcessingEvent::RecordingStatus(message) => {
                    self.recording_messages.push(message);
                    const MAX_RECORDING_MESSAGES: usize = 6;
                    if self.recording_messages.len() > MAX_RECORDING_MESSAGES {
                        self.recording_messages
                            .drain(0..self.recording_messages.len() - MAX_RECORDING_MESSAGES);
                    }
                }
                ProcessingEvent::Complete => {
                    self.processing_step = ProcessingStep::Complete;
                    self.processing_rx = None;
                    self.reload_sessions();
                    self.screen = Screen::Complete;
                }
                ProcessingEvent::Error(message) => {
                    tracing::error!(error = %message, "TUI processing failed");
                    self.message = message;
                    self.processing_rx = None;
                    self.reload_sessions();
                    self.screen = Screen::Error;
                }
            }
        }
    }
}

fn spawn_processing_task(
    tx: mpsc::UnboundedSender<ProcessingEvent>,
    cfg: config::Config,
    recording: RecordingState,
) {
    tokio::spawn(async move {
        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::Finalizing));
        tracing::info!("TUI processing finalizing recording");
        match recording.task.await {
            Ok(Ok(())) => {
                tracing::info!("TUI recording finalized");
            }
            Ok(Err(error)) => {
                tracing::error!(error = %error, "TUI recording finalization failed");
                let _ = tx.send(ProcessingEvent::Error(error.to_string()));
                return;
            }
            Err(error) => {
                tracing::error!(error = %error, "TUI recording task failed to join");
                let _ = tx.send(ProcessingEvent::Error(format!(
                    "Recording task failed to join: {error}"
                )));
                return;
            }
        }

        let session_dir = recording.session_dir;
        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::Transcribing));
        let wav_path = session_dir.join("recording.wav");
        tracing::info!(
            session_dir = %session_dir.display(),
            wav_path = %wav_path.display(),
            "TUI transcription starting"
        );
        let transcript = match transcribe::run_whisper(&wav_path, &cfg).await {
            Ok(transcript) => transcript,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    session_dir = %session_dir.display(),
                    wav_path = %wav_path.display(),
                    "TUI transcription failed"
                );
                let _ = tx.send(ProcessingEvent::Error(error.to_string()));
                return;
            }
        };
        tracing::info!(
            session_dir = %session_dir.display(),
            transcript_chars = transcript.len(),
            "TUI transcription completed"
        );

        let txt_path = session_dir.join("transcript.txt");
        if let Err(error) = std::fs::write(&txt_path, &transcript) {
            tracing::error!(
                error = %error,
                transcript_path = %txt_path.display(),
                "TUI transcript write failed"
            );
            let _ = tx.send(ProcessingEvent::Error(error.to_string()));
            return;
        }
        tracing::info!(transcript_path = %txt_path.display(), "TUI transcript saved");

        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::GeneratingNotes));
        tracing::info!(session_dir = %session_dir.display(), "TUI notes generation starting");
        let notes_generator = notes::OpenRouterNotesGenerator::from_config(&cfg);
        let notes_input = notes::NoteGenerationInput {
            transcript: transcript.clone(),
            context: notes::NoteGenerationContext {
                note_date: chrono::Local::now().format("%B %-d, %Y").to_string(),
                system_prompt: notes::NotesSystemPrompt::Default,
            },
        };
        let notes_text = match notes_generator.generate(notes_input).await {
            Ok(notes_output) => notes_output.markdown,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    session_dir = %session_dir.display(),
                    "TUI notes generation failed"
                );
                let _ = tx.send(ProcessingEvent::Error(error.to_string()));
                return;
            }
        };
        tracing::info!(session_dir = %session_dir.display(), "TUI notes generation completed");

        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::WritingNotes));
        let full_notes = format!("{notes_text}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
        let notes_path = session_dir.join("notes.md");
        if let Err(error) = std::fs::write(&notes_path, &full_notes) {
            tracing::error!(
                error = %error,
                notes_path = %notes_path.display(),
                "TUI notes write failed"
            );
            let _ = tx.send(ProcessingEvent::Error(error.to_string()));
            return;
        }
        tracing::info!(notes_path = %notes_path.display(), "TUI notes saved");

        let _ = tx.send(ProcessingEvent::Complete);
    });
}

fn cleanup_archive_on_startup() {
    match config::config_dir() {
        Ok(config_dir) => {
            let archive_root = session_store::archive_dir(&config_dir);
            match session_store::cleanup_archive(
                &archive_root,
                Duration::from_secs(7 * 24 * 60 * 60),
                SystemTime::now(),
            ) {
                Ok(removed) => {
                    tracing::info!(
                        archive_dir = %archive_root.display(),
                        removed,
                        "TUI archive cleanup completed"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        archive_dir = %archive_root.display(),
                        error = %error,
                        "TUI archive cleanup failed"
                    );
                }
            }
        }
        Err(error) => tracing::warn!(error = %error, "TUI archive cleanup skipped"),
    }
}

impl SetupForm {
    fn from_config(cfg: Option<&config::Config>) -> Self {
        let default_cfg = config::config_dir()
            .map(|dir| config::default_config(&dir))
            .unwrap_or_else(|_| config::Config {
                whisper_bin: default_setup_whisper_bin(),
                whisper_model: "ggml-base.en.bin".to_string(),
                openrouter_api_key: "YOUR_KEY_HERE".to_string(),
                model: "google/gemini-2.5-flash".to_string(),
                sample_rate: 16000,
                output_dir: None,
            });
        let cfg = cfg.unwrap_or(&default_cfg);
        let output_dir = cfg.output_dir.clone().or_else(|| {
            config::effective_output_dir(cfg)
                .ok()
                .map(|p| p.to_string_lossy().into())
        });

        Self {
            openrouter_api_key: cfg.openrouter_api_key.clone(),
            model: cfg.model.clone(),
            whisper_bin: cfg.whisper_bin.clone().unwrap_or_default(),
            whisper_model: cfg.whisper_model.clone(),
            output_dir: output_dir.unwrap_or_default(),
            focus: SetupFocus::ApiKey,
            message: String::new(),
        }
    }

    fn to_config(&self) -> Result<config::Config> {
        Ok(config::Config {
            whisper_bin: if self.whisper_bin.trim().is_empty() {
                None
            } else {
                Some(self.whisper_bin.trim().to_string())
            },
            whisper_model: self.whisper_model.trim().to_string(),
            openrouter_api_key: self.openrouter_api_key.trim().to_string(),
            model: self.model.trim().to_string(),
            sample_rate: 16000,
            output_dir: if self.output_dir.trim().is_empty() {
                None
            } else {
                Some(self.output_dir.trim().to_string())
            },
        })
    }

    fn push_char(&mut self, ch: char) {
        match self.focus {
            SetupFocus::ApiKey => self.openrouter_api_key.push(ch),
            SetupFocus::NotesModel => self.model.push(ch),
            SetupFocus::WhisperBin => self.whisper_bin.push(ch),
            SetupFocus::WhisperModel => self.whisper_model.push(ch),
            SetupFocus::OutputDir => self.output_dir.push(ch),
            _ => {}
        }
    }

    fn delete_char(&mut self) {
        match self.focus {
            SetupFocus::ApiKey => {
                self.openrouter_api_key.pop();
            }
            SetupFocus::NotesModel => {
                self.model.pop();
            }
            SetupFocus::WhisperBin => {
                self.whisper_bin.pop();
            }
            SetupFocus::WhisperModel => {
                self.whisper_model.pop();
            }
            SetupFocus::OutputDir => {
                self.output_dir.pop();
            }
            _ => {}
        }
    }
}

fn default_setup_whisper_bin() -> Option<String> {
    #[cfg(feature = "whisper-cli")]
    {
        Some("whisper-cli".to_string())
    }

    #[cfg(not(feature = "whisper-cli"))]
    {
        None
    }
}

fn next_setup_focus(focus: SetupFocus) -> SetupFocus {
    match focus {
        SetupFocus::ApiKey => SetupFocus::NotesModel,
        SetupFocus::NotesModel => SetupFocus::WhisperBin,
        SetupFocus::WhisperBin => SetupFocus::WhisperModel,
        SetupFocus::WhisperModel => SetupFocus::OutputDir,
        SetupFocus::OutputDir => SetupFocus::Validate,
        SetupFocus::Validate => SetupFocus::Download,
        SetupFocus::Download => SetupFocus::Save,
        SetupFocus::Save => SetupFocus::Quit,
        SetupFocus::Quit => SetupFocus::ApiKey,
    }
}

fn previous_setup_focus(focus: SetupFocus) -> SetupFocus {
    match focus {
        SetupFocus::ApiKey => SetupFocus::Quit,
        SetupFocus::NotesModel => SetupFocus::ApiKey,
        SetupFocus::WhisperBin => SetupFocus::NotesModel,
        SetupFocus::WhisperModel => SetupFocus::WhisperBin,
        SetupFocus::OutputDir => SetupFocus::WhisperModel,
        SetupFocus::Validate => SetupFocus::OutputDir,
        SetupFocus::Download => SetupFocus::Validate,
        SetupFocus::Save => SetupFocus::Download,
        SetupFocus::Quit => SetupFocus::Save,
    }
}

fn render(frame: &mut Frame<'_>, app: &App) {
    match app.screen {
        Screen::Setup => render_setup(frame, app),
        Screen::Sessions => render_sessions(frame, app),
        Screen::SessionDetail => render_session_detail(frame, app),
        Screen::TextViewer => render_text_viewer(frame, app),
        Screen::Playback => render_playback(frame, app),
        Screen::EditSessionName => {
            render_session_detail(frame, app);
            render_edit_session_name(frame, app);
        }
        Screen::NewRecording => {
            render_sessions(frame, app);
            render_new_recording(frame, app);
        }
        Screen::Recording => render_recording(frame, app),
        Screen::Processing => render_processing(frame, app),
        Screen::Complete => render_complete(frame, app),
        Screen::Error => render_error(frame, app),
    }
}

fn render_setup(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(88, 70, frame.area());
    let masked_key = mask_key(&app.setup.openrouter_api_key);
    let lines = vec![
        field_line(
            "OpenRouter API key",
            &masked_key,
            app.setup.focus == SetupFocus::ApiKey,
        ),
        field_line(
            "Notes model",
            &app.setup.model,
            app.setup.focus == SetupFocus::NotesModel,
        ),
        Line::from(format!("Whisper backend    {}", whisper_backend_label())),
        field_line(
            "whisper_bin",
            &app.setup.whisper_bin,
            app.setup.focus == SetupFocus::WhisperBin,
        ),
        field_line(
            "Whisper model",
            &app.setup.whisper_model,
            app.setup.focus == SetupFocus::WhisperModel,
        ),
        field_line(
            "Output folder",
            &app.setup.output_dir,
            app.setup.focus == SetupFocus::OutputDir,
        ),
        Line::from(format!(
            "Model status       {}",
            model_status(&app.setup.whisper_model)
        )),
        Line::from(""),
        actions_line(&[
            ("Validate", app.setup.focus == SetupFocus::Validate),
            ("Download model", app.setup.focus == SetupFocus::Download),
            ("Save setup", app.setup.focus == SetupFocus::Save),
            ("Quit", app.setup.focus == SetupFocus::Quit),
        ]),
        Line::from(""),
        Line::from(app.setup.message.clone()),
        Line::from(""),
        Line::from("Tab moves focus. Enter activates. Esc quits."),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Scribe Setup ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
    if let Some((line_index, value)) = setup_cursor_value(&app.setup, &masked_key) {
        frame.set_cursor_position(input_cursor_position(area, line_index, 18, value));
    }
}

fn whisper_backend_label() -> &'static str {
    #[cfg(feature = "whisper-cli")]
    {
        "whisper.cpp CLI"
    }

    #[cfg(not(feature = "whisper-cli"))]
    {
        "embedded whisper.cpp"
    }
}

fn render_sessions(frame: &mut Frame<'_>, app: &App) {
    let layout = split_work_screen(frame.area());

    let sessions = if app.sessions.is_empty() {
        vec![ListItem::new("No sessions yet.")]
    } else {
        app.sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                let prefix = if index == app.selected_session {
                    "> "
                } else {
                    "  "
                };
                let status = session_status_text(&session.status);
                let modified = format_time(session.modified);
                ListItem::new(vec![
                    Line::from(format!("{prefix}{}", session.name)),
                    Line::from(format!("  {status} - modified {modified}")),
                    Line::from(""),
                ])
            })
            .collect()
    };

    frame.render_widget(
        List::new(sessions).block(Block::default().title(" Sessions ").borders(Borders::ALL)),
        layout.main,
    );

    let output = app
        .cfg
        .as_ref()
        .and_then(|cfg| config::effective_output_dir(cfg).ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let actions = session_action_texts()
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(actions).block(Block::default().title(" Actions ").borders(Borders::ALL)),
        layout.actions,
    );
    render_footer(
        frame,
        layout.footer,
        footer_text(format!("Output: {output}"), &app.message),
    );
}

fn render_session_detail(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let layout = split_work_screen(area);
    let Some(session) = &app.detail_session else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from("No session selected."),
                Line::from("[ Esc ] Back"),
            ])
            .block(
                Block::default()
                    .title(" Session Detail ")
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    };

    let recorded = session
        .recorded_at
        .map(format_time)
        .unwrap_or_else(|| "Unknown".to_string());
    let recording_path = session.path.join("recording.wav");
    let notes_path = session.path.join("notes.md");
    let transcript_path = session.path.join("transcript.txt");
    let details = vec![
        Line::from(format!("Session       {}", session.name)),
        Line::from(format!("Recorded      {recorded}")),
        Line::from(format!(
            "Status        {}",
            session_status_text(&session.status)
        )),
        Line::from(format!("Directory     {}", session.path.display())),
        Line::from(""),
        Line::from(format!(
            "Notes         {}",
            available_text(notes_path.exists())
        )),
        Line::from(format!(
            "Transcript    {}",
            available_text(transcript_path.exists())
        )),
        Line::from(format!(
            "Recording     {}",
            available_text(recording_path.exists())
        )),
    ];
    frame.render_widget(
        Paragraph::new(details)
            .block(
                Block::default()
                    .title(" Session Detail ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true }),
        layout.main,
    );

    let actions = DETAIL_ACTIONS
        .iter()
        .enumerate()
        .map(|(index, action)| {
            detail_action_line(*action, index == app.selected_detail_action, session)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(actions).block(Block::default().title(" Actions ").borders(Borders::ALL)),
        layout.actions,
    );
    render_footer(
        frame,
        layout.footer,
        footer_text(
            app.message.clone(),
            confirmation_text(app.pending_session_action),
        ),
    );
}

fn render_text_viewer(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let Some(viewer) = &app.text_viewer else {
        frame.render_widget(
            Paragraph::new("[ Esc ] Back").block(
                Block::default()
                    .title(" Text Viewer ")
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    };

    let inner_height = area.height.saturating_sub(3).max(1) as usize;
    let end = (viewer.scroll + inner_height).min(viewer.lines.len());
    let visible = viewer.lines[viewer.scroll..end]
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();
    let footer = format!(
        "{} | lines {}-{} of {} | j/k scroll | PgUp/PgDn page | g/G ends | Esc back",
        viewer.path.display(),
        if viewer.lines.is_empty() {
            0
        } else {
            viewer.scroll + 1
        },
        end,
        viewer.lines.len()
    );
    frame.render_widget(
        Paragraph::new(visible)
            .block(
                Block::default()
                    .title(format!(" {} ", viewer.title))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
    let footer_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(1),
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(Paragraph::new(footer), footer_area);
}

fn render_playback(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(82, 45, frame.area());
    let Some(playback) = &app.playback else {
        frame.render_widget(
            Paragraph::new("[ Esc ] Back")
                .block(Block::default().title(" Playback ").borders(Borders::ALL)),
            area,
        );
        return;
    };

    let status = if playback.is_playing() {
        "Playing"
    } else {
        &playback.status
    };
    let position = format_duration(playback.position());
    let duration = playback
        .duration()
        .map(format_duration)
        .unwrap_or_else(|| "--:--:--".to_string());
    let mut lines = vec![
        Line::from(format!("Session     {}", playback.session_name)),
        Line::from(format!("Audio file  {}", playback.path.display())),
        Line::from(format!("Position    {position} / {duration}")),
        Line::from(format!("Status      {status}")),
        Line::from(""),
        Line::from("[ Space ] Play/Pause    [ r ] Restart    [ s ] Stop"),
        Line::from("[ h ] -10s              [ l ] +30s"),
        Line::from("[ Esc ] Back"),
    ];
    if let Some(error) = &playback.error {
        lines.extend([Line::from(""), Line::from(format!("Error: {error}"))]);
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(" Playback ").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_new_recording(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(52, 28, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Session name"),
            Line::from(format!("[ {} ]", app.recording_name)),
            Line::from(""),
            Line::from(app.message.clone()),
            Line::from(""),
            Line::from("[ Enter ] Start recording    [ Esc ] Cancel"),
        ])
        .block(
            Block::default()
                .title(" New Recording ")
                .borders(Borders::ALL),
        ),
        area,
    );
    frame.set_cursor_position(input_cursor_position(area, 1, 0, &app.recording_name));
}

fn render_edit_session_name(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(52, 28, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Session name"),
            Line::from(format!("[ {} ]", app.recording_name)),
            Line::from(""),
            Line::from(app.message.clone()),
            Line::from(""),
            Line::from("[ Enter ] Save name    [ Esc ] Cancel"),
        ])
        .block(
            Block::default()
                .title(" Rename Session ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true }),
        area,
    );
    frame.set_cursor_position(input_cursor_position(area, 1, 0, &app.recording_name));
}

fn render_recording(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(82, 55, frame.area());
    let recording = app.recording.as_ref();
    let elapsed = recording
        .map(|state| format_duration(state.started_at.elapsed()))
        .unwrap_or_else(|| "00:00:00".to_string());
    let session = recording
        .map(|state| state.session_name.clone())
        .unwrap_or_default();
    let destination = recording
        .map(|state| state.session_dir.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut lines = vec![
        Line::from("* Recording"),
        Line::from(""),
        Line::from(format!("Session      {session}")),
        Line::from(format!("Elapsed      {elapsed}")),
        Line::from(format!("Destination  {destination}")),
        Line::from("Audio file   recording.wav"),
        Line::from(""),
        Line::from("Audio status"),
    ];
    if app.recording_messages.is_empty() {
        lines.push(Line::from("Waiting for audio device details..."));
    } else {
        lines.extend(app.recording_messages.iter().cloned().map(Line::from));
    }
    lines.extend([Line::from(""), Line::from("[ Enter ] Stop & Process")]);

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Scribe Recording ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_processing(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(76, 45, frame.area());
    frame.render_widget(
        Paragraph::new(processing_lines(app.processing_step)).block(
            Block::default()
                .title(" Processing recording ")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_complete(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(76, 35, frame.area());
    let path = app
        .processing_session
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("[x] Recording finalized"),
            Line::from("[x] Transcript saved"),
            Line::from("[x] Notes saved"),
            Line::from(""),
            Line::from(path),
            Line::from(""),
            Line::from("[ v ] View    [ b ] Back to sessions    [ q ] Quit"),
        ])
        .block(
            Block::default()
                .title(" Processing complete ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_error(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(76, 35, frame.area());
    let log_path = app
        .log_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unavailable".to_string());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Processing failed"),
            Line::from(""),
            Line::from(format!("Error: {}", app.message)),
            Line::from(""),
            Line::from(format!("Log: {log_path}")),
            Line::from(""),
            Line::from("[ b ] Back to sessions    [ q ] Quit"),
        ])
        .block(Block::default().title(" Error ").borders(Borders::ALL))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn split_work_screen(area: Rect) -> WorkScreenLayout {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(FOOTER_HEIGHT)])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(45), Constraint::Length(ACTION_PANEL_WIDTH)])
        .split(vertical[0]);
    WorkScreenLayout {
        main: top[0],
        actions: top[1],
        footer: vertical[1],
    }
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, text: String) {
    frame.render_widget(
        Paragraph::new(text).block(Block::default().title(" Messages ").borders(Borders::ALL)),
        area,
    );
}

fn footer_text(primary: impl Into<String>, secondary: &str) -> String {
    let primary = primary.into();
    if primary.is_empty() {
        secondary.to_string()
    } else if secondary.is_empty() {
        primary
    } else {
        format!("{primary}    {secondary}")
    }
}

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::raw(format!("{label:<18}")),
        Span::styled(format!("[ {value} ]"), style),
    ])
}

fn setup_cursor_value<'a>(setup: &'a SetupForm, masked_key: &'a str) -> Option<(usize, &'a str)> {
    match setup.focus {
        SetupFocus::ApiKey => Some((0, masked_key)),
        SetupFocus::NotesModel => Some((1, setup.model.as_str())),
        SetupFocus::WhisperBin => Some((3, setup.whisper_bin.as_str())),
        SetupFocus::WhisperModel => Some((4, setup.whisper_model.as_str())),
        SetupFocus::OutputDir => Some((5, setup.output_dir.as_str())),
        _ => None,
    }
}

fn input_cursor_position(
    area: Rect,
    line_index: usize,
    field_prefix_width: u16,
    value: &str,
) -> (u16, u16) {
    let value_width = UnicodeWidthStr::width(value) as u16;
    let inner_left = area.x.saturating_add(1);
    let inner_right = area.x.saturating_add(area.width.saturating_sub(2));
    let x = inner_left
        .saturating_add(field_prefix_width)
        .saturating_add(2)
        .saturating_add(value_width)
        .min(inner_right);
    let y = area.y.saturating_add(1).saturating_add(line_index as u16);
    (x, y)
}

fn actions_line(actions: &[(&str, bool)]) -> Line<'static> {
    let spans = actions
        .iter()
        .flat_map(|(label, focused)| {
            let style = if *focused {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            [
                Span::styled(format!("[ {label} ]"), style),
                Span::raw("  ".to_string()),
            ]
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn session_action_texts() -> [&'static str; 5] {
    [
        "[ r ] Record",
        "[ f ] Refresh",
        "[ s ] Settings",
        "[ o ] Details",
        "[ q ] Quit",
    ]
}

fn mask_key(key: &str) -> String {
    if key.is_empty() || key == "YOUR_KEY_HERE" {
        key.to_string()
    } else {
        "*".repeat(key.chars().count().min(32))
    }
}

fn model_status(path: &str) -> &'static str {
    if PathBuf::from(path).exists() {
        "present"
    } else {
        "missing"
    }
}

fn detail_action_line(
    action: DetailAction,
    selected: bool,
    session: &sessions::SessionEntry,
) -> Line<'static> {
    let (label, shortcut, enabled) = match action {
        DetailAction::Notes => ("Notes", "n", session.path.join("notes.md").exists()),
        DetailAction::Transcript => (
            "Transcript",
            "t",
            session.path.join("transcript.txt").exists(),
        ),
        DetailAction::Playback => ("Playback", "p", session.path.join("recording.wav").exists()),
        DetailAction::OpenFolder => ("Open Folder", "o", true),
        DetailAction::Rename => ("Rename", "e", true),
        DetailAction::Archive => ("Archive", "a", true),
        DetailAction::Delete => ("Delete", "d", true),
    };
    let prefix = if selected { "> " } else { "  " };
    let state = if enabled { "" } else { " (missing)" };
    let style = if selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if enabled {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Line::from(Span::styled(
        format!("{prefix}[ {shortcut} ] {label}{state}"),
        style,
    ))
}

fn confirmation_text(action: Option<PendingSessionAction>) -> &'static str {
    match action {
        Some(PendingSessionAction::Archive) => "Confirm archive: [ y ] Yes  [ n ] No",
        Some(PendingSessionAction::Delete) => "Confirm delete: [ y ] Yes  [ n ] No",
        None => "",
    }
}

fn available_text(available: bool) -> &'static str {
    if available { "available" } else { "missing" }
}

fn session_status_text(status: &audio::SessionStatus) -> &'static str {
    match status {
        audio::SessionStatus::Empty => "empty",
        audio::SessionStatus::RecordingOnly => "recording.wav",
        audio::SessionStatus::TranscriptReady => "transcript.txt + recording.wav",
        audio::SessionStatus::NotesReady => "notes.md + transcript.txt + recording.wav",
    }
}

fn processing_lines(step: ProcessingStep) -> Vec<Line<'static>> {
    [
        (ProcessingStep::Finalizing, "Recording finalized"),
        (
            ProcessingStep::Transcribing,
            "Transcribing with whisper.cpp",
        ),
        (ProcessingStep::GeneratingNotes, "Generating notes"),
        (ProcessingStep::WritingNotes, "Writing notes.md"),
    ]
    .into_iter()
    .map(|(candidate, label)| {
        let marker = if step as u8 > candidate as u8 {
            "[x]"
        } else if step == candidate {
            "->"
        } else {
            " ."
        };
        Line::from(format!("{marker} {label}"))
    })
    .collect()
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

fn format_time(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

fn editable_session_name(name: &str) -> String {
    if sessions::recorded_at_from_session_name(name).is_some()
        && let Some(prefix) = name.get(..17)
    {
        let user_prefix = format!("{prefix} — ");
        return name.strip_prefix(&user_prefix).unwrap_or("").to_string();
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_focus_cycles_forward_and_backward() {
        assert_eq!(next_setup_focus(SetupFocus::ApiKey), SetupFocus::NotesModel);
        assert_eq!(previous_setup_focus(SetupFocus::ApiKey), SetupFocus::Quit);
    }

    #[test]
    fn setup_form_converts_empty_whisper_bin_to_none() {
        let mut form = SetupForm::from_config(None);
        form.openrouter_api_key = "sk-or-test".to_string();
        form.whisper_bin.clear();

        let cfg = form.to_config().unwrap();

        assert!(cfg.whisper_bin.is_none());
    }

    #[test]
    fn escape_from_initial_setup_quits_tui() {
        let mut app = test_app(Screen::Setup);

        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(quit);
        assert_eq!(app.screen, Screen::Setup);
    }

    #[test]
    fn escape_from_in_app_settings_returns_to_previous_screen() {
        let mut app = test_app(Screen::Sessions);

        tokio_test_handle_key(&mut app, KeyCode::Char('s')).unwrap();
        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::Sessions);
    }

    #[test]
    fn recording_prompt_cancel_returns_to_sessions() {
        let mut app = test_app(Screen::NewRecording);
        app.recording_name = "Standup".to_string();

        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::Sessions);
    }

    #[test]
    fn processing_complete_event_moves_to_complete_screen() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = test_app(Screen::Processing);
        app.processing_rx = Some(rx);

        tx.send(ProcessingEvent::Complete).unwrap();
        app.drain_processing_events();

        assert_eq!(app.screen, Screen::Complete);
        assert_eq!(app.processing_step, ProcessingStep::Complete);
        assert!(app.processing_rx.is_none());
    }

    #[test]
    fn recording_status_event_updates_visible_messages() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = test_app(Screen::Recording);
        app.processing_rx = Some(rx);

        tx.send(ProcessingEvent::RecordingStatus(
            "Loopback: HD Pro Webcam C920".to_string(),
        ))
        .unwrap();
        app.drain_processing_events();

        assert_eq!(app.recording_messages, vec!["Loopback: HD Pro Webcam C920"]);
    }

    #[test]
    fn processing_error_event_preserves_message_and_log_path() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = test_app(Screen::Processing);
        app.log_path = Some(PathBuf::from("/tmp/scribe.log"));
        app.processing_step = ProcessingStep::Transcribing;
        app.processing_rx = Some(rx);

        tx.send(ProcessingEvent::Error(
            "whisper.cpp failed: missing model".to_string(),
        ))
        .unwrap();
        app.drain_processing_events();

        assert_eq!(app.screen, Screen::Error);
        assert_eq!(app.message, "whisper.cpp failed: missing model");
        assert_eq!(app.log_path, Some(PathBuf::from("/tmp/scribe.log")));
        assert!(app.processing_rx.is_none());
    }

    #[test]
    fn enter_on_sessions_opens_session_detail() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();

        let mut app = test_app(Screen::Sessions);
        app.sessions = vec![sessions::SessionEntry {
            path: session.clone(),
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::Empty,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name("2026-05-08_164949 — Test 1"),
        }];

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::SessionDetail);
        assert_eq!(app.detail_session.as_ref().map(|s| &s.path), Some(&session));
    }

    #[test]
    fn escape_from_detail_returns_to_sessions() {
        let mut app = test_app(Screen::SessionDetail);
        app.detail_session = Some(test_session_entry("2026-05-08_164949 — Test 1"));

        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::Sessions);
    }

    #[test]
    fn detail_opens_notes_text_viewer() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("notes.md"), "line 1\nline 2").unwrap();

        let mut app = test_app(Screen::SessionDetail);
        app.detail_session = Some(sessions::SessionEntry {
            path: session,
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::NotesReady,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name("2026-05-08_164949 — Test 1"),
        });

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::TextViewer);
        let viewer = app.text_viewer.as_ref().unwrap();
        assert_eq!(viewer.title, "Notes");
        assert_eq!(viewer.lines, vec!["line 1", "line 2"]);
    }

    #[test]
    fn missing_notes_preserves_error_message_on_detail_screen() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();

        let mut app = test_app(Screen::SessionDetail);
        app.detail_session = Some(sessions::SessionEntry {
            path: session,
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::RecordingOnly,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name("2026-05-08_164949 — Test 1"),
        });

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::SessionDetail);
        assert_eq!(app.message, "notes.md is not available for this session.");
    }

    #[test]
    fn new_recording_rejects_empty_session_name() {
        let mut app = test_app(Screen::NewRecording);
        app.recording_name = "   ".to_string();

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::NewRecording);
        assert_eq!(app.message, "Session name cannot be empty");
    }

    #[test]
    fn new_recording_rejects_non_printable_session_name() {
        let mut app = test_app(Screen::NewRecording);
        app.recording_name = "Team\tStandup".to_string();

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::NewRecording);
        assert_eq!(
            app.message,
            "Session name must use printable Unicode characters"
        );
    }

    #[test]
    fn escape_from_text_viewer_returns_to_detail() {
        let mut app = test_app(Screen::TextViewer);
        app.text_viewer = Some(TextViewerState::new(
            "Transcript",
            PathBuf::from("transcript.txt"),
            vec!["hello".to_string()],
        ));

        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::SessionDetail);
    }

    #[test]
    fn escape_from_playback_returns_to_detail() {
        let mut app = test_app(Screen::Playback);
        app.playback = Some(playback::PlaybackViewState {
            session_name: "2026-05-08_164949 — Test 1".to_string(),
            path: PathBuf::from("recording.wav"),
            controller: None,
            status: "Ready".to_string(),
            error: None,
        });

        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::SessionDetail);
        assert_eq!(app.playback.as_ref().unwrap().status, "Stopped");
    }

    #[test]
    fn detail_delete_requires_confirmation_and_removes_session() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();

        let mut app = test_app(Screen::SessionDetail);
        app.detail_session = Some(sessions::SessionEntry {
            path: session.clone(),
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::RecordingOnly,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name("2026-05-08_164949 — Test 1"),
        });

        tokio_test_handle_key(&mut app, KeyCode::Char('d')).unwrap();
        assert_eq!(
            app.pending_session_action,
            Some(PendingSessionAction::Delete)
        );
        assert!(session.exists());

        tokio_test_handle_key(&mut app, KeyCode::Char('y')).unwrap();

        assert!(!session.exists());
        assert_eq!(app.screen, Screen::Sessions);
        assert_eq!(app.detail_session, None);
    }

    #[test]
    fn detail_archive_moves_session_to_archive_root() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        let archive = temp.path().join("config").join("archive");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("transcript.txt"), "hello").unwrap();

        let mut app = test_app(Screen::SessionDetail);
        app.detail_session = Some(sessions::SessionEntry {
            path: session.clone(),
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::TranscriptReady,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name("2026-05-08_164949 — Test 1"),
        });

        app.archive_detail_session_to(&archive).unwrap();

        assert!(!session.exists());
        assert!(archive.join("2026-05-08_164949 — Test 1").exists());
        assert_eq!(app.screen, Screen::Sessions);
        assert_eq!(app.detail_session, None);
    }

    #[test]
    fn detail_rename_opens_edit_prompt_with_user_visible_name() {
        let mut app = test_app(Screen::SessionDetail);
        app.detail_session = Some(test_session_entry("2026-05-08_164949 — Test 1"));

        let quit = tokio_test_handle_key(&mut app, KeyCode::Char('e')).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::EditSessionName);
        assert_eq!(app.recording_name, "Test 1");
    }

    #[test]
    fn edit_session_name_renames_folder_and_returns_to_detail() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();

        let mut app = test_app(Screen::EditSessionName);
        app.detail_session = Some(sessions::SessionEntry {
            path: session.clone(),
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::Empty,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name("2026-05-08_164949 — Test 1"),
        });
        app.recording_name = "Team standup".to_string();

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::SessionDetail);
        let renamed = temp.path().join("2026-05-08_164949 — Team standup");
        assert!(!session.exists());
        assert!(renamed.exists());
        let detail = app.detail_session.as_ref().unwrap();
        assert_eq!(detail.path, renamed);
        assert_eq!(detail.name, "2026-05-08_164949 — Team standup");
    }

    #[test]
    fn edit_session_name_reuses_session_name_validation() {
        let mut app = test_app(Screen::EditSessionName);
        app.detail_session = Some(test_session_entry("2026-05-08_164949 — Test 1"));
        app.recording_name = "\u{7}".to_string();

        let quit = tokio_test_handle_key(&mut app, KeyCode::Enter).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::EditSessionName);
        assert_eq!(
            app.message,
            "Session name must use printable Unicode characters"
        );
    }

    #[test]
    fn pending_detail_action_can_be_cancelled() {
        let mut app = test_app(Screen::SessionDetail);
        app.request_session_action(PendingSessionAction::Archive);

        tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert_eq!(app.pending_session_action, None);
        assert_eq!(app.message, "Cancelled.");
        assert_eq!(app.screen, Screen::SessionDetail);
    }

    #[test]
    fn text_viewer_scroll_clamps_at_bounds() {
        let mut viewer = TextViewerState::new(
            "Transcript",
            PathBuf::from("transcript.txt"),
            (0..5).map(|line| format!("line {line}")).collect(),
        );

        viewer.scroll_down(10);
        assert_eq!(viewer.scroll, 4);

        viewer.scroll_up(10);
        assert_eq!(viewer.scroll, 0);
    }

    #[test]
    fn playback_seek_helpers_clamp_to_bounds() {
        assert_eq!(
            playback::rewind_position(Duration::from_secs(5), Duration::from_secs(10)),
            Duration::ZERO
        );
        assert_eq!(
            playback::fast_forward_position(
                Duration::from_secs(80),
                Duration::from_secs(30),
                Some(Duration::from_secs(90)),
            ),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn input_cursor_position_points_after_value() {
        let area = Rect::new(10, 5, 80, 12);

        let position = input_cursor_position(area, 1, 18, "Standup");

        assert_eq!(position, (38, 7));
    }

    #[test]
    fn input_cursor_position_clamps_to_inner_width() {
        let area = Rect::new(0, 0, 24, 5);

        let position = input_cursor_position(area, 0, 18, "long session name");

        assert_eq!(position, (22, 1));
    }

    #[test]
    fn work_screen_layout_uses_wider_action_panel_and_footer() {
        let area = Rect::new(0, 0, 100, 40);

        let layout = split_work_screen(area);

        assert_eq!(layout.main, Rect::new(0, 0, 68, 37));
        assert_eq!(layout.actions, Rect::new(68, 0, ACTION_PANEL_WIDTH, 37));
        assert_eq!(layout.footer, Rect::new(0, 37, 100, FOOTER_HEIGHT));
    }

    #[test]
    fn session_action_texts_use_single_key_shortcuts() {
        let actions = session_action_texts();

        assert!(actions.contains(&"[ o ] Details"));
        assert!(actions.iter().all(|action| !action.contains('/')));
        assert!(
            actions
                .iter()
                .all(|action| UnicodeWidthStr::width(*action) <= ACTION_PANEL_WIDTH as usize - 2)
        );
    }

    fn test_app(screen: Screen) -> App {
        App {
            screen,
            setup_return_screen: None,
            cfg: None,
            log_path: None,
            setup: SetupForm::from_config(None),
            sessions: Vec::new(),
            selected_session: 0,
            recording_name: String::new(),
            recording: None,
            recording_messages: Vec::new(),
            processing_session: None,
            processing_step: ProcessingStep::Finalizing,
            processing_rx: None,
            message: String::new(),
            selected_detail_action: 0,
            detail_session: None,
            text_viewer: None,
            playback: None,
            pending_session_action: None,
        }
    }

    fn test_session_entry(name: &str) -> sessions::SessionEntry {
        sessions::SessionEntry {
            path: PathBuf::from(name),
            name: name.to_string(),
            status: audio::SessionStatus::Empty,
            modified: SystemTime::UNIX_EPOCH,
            recorded_at: sessions::recorded_at_from_session_name(name),
        }
    }

    fn tokio_test_handle_key(app: &mut App, code: KeyCode) -> Result<bool> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(handle_key(app, KeyEvent::from(code)))
    }
}
