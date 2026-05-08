use crate::{audio, config, notes, opener, transcribe};
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
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Screen {
    Setup,
    Sessions,
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
    task: tokio::task::JoinHandle<()>,
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
    Complete,
    Error(String),
}

struct App {
    screen: Screen,
    cfg: Option<config::Config>,
    setup: SetupForm,
    sessions: Vec<audio::SessionEntry>,
    selected_session: usize,
    recording_name: String,
    recording: Option<RecordingState>,
    processing_session: Option<PathBuf>,
    processing_step: ProcessingStep,
    processing_rx: Option<mpsc::UnboundedReceiver<ProcessingEvent>>,
    message: String,
}

pub async fn run() -> Result<()> {
    let mut app = App::load()?;
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &mut app).await;
    restore_terminal(&mut terminal)?;
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
        Screen::NewRecording => handle_new_recording_key(app, key).await,
        Screen::Recording => handle_recording_key(app, key).await,
        Screen::Processing => Ok(false),
        Screen::Complete => handle_complete_key(app, key),
        Screen::Error => handle_error_key(app, key),
    }
}

async fn handle_setup_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => return Ok(true),
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
        KeyCode::Enter | KeyCode::Char('o') => app.open_selected_session()?,
        KeyCode::Char('r') => {
            app.recording_name.clear();
            app.screen = Screen::NewRecording;
        }
        KeyCode::Char('s') => {
            app.setup = SetupForm::from_config(app.cfg.as_ref());
            app.screen = Screen::Setup;
        }
        KeyCode::Char('f') => app.reload_sessions(),
        _ => {}
    }
    Ok(false)
}

async fn handle_new_recording_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.screen = Screen::Sessions,
        KeyCode::Backspace => {
            app.recording_name.pop();
        }
        KeyCode::Char(ch) => app.recording_name.push(ch),
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
    fn load() -> Result<Self> {
        let cfg = config::load_existing()?;
        let setup = SetupForm::from_config(cfg.as_ref());
        let mut app = Self {
            screen: Screen::Setup,
            cfg,
            setup,
            sessions: Vec::new(),
            selected_session: 0,
            recording_name: String::new(),
            recording: None,
            processing_session: None,
            processing_step: ProcessingStep::Finalizing,
            processing_rx: None,
            message: String::new(),
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
            Ok(_) => self.setup.message = "Setup looks valid.".to_string(),
            Err(error) => self.setup.message = error.to_string(),
        }
    }

    async fn download_model(&mut self) {
        self.setup.message = "Downloading model...".to_string();
        match config::ensure_managed_whisper_model().await {
            Ok(path) => {
                self.setup.whisper_model = path.to_string_lossy().into_owned();
                self.setup.message = "Model downloaded.".to_string();
            }
            Err(error) => self.setup.message = error.to_string(),
        }
    }

    fn save_setup(&mut self) -> Result<()> {
        let config_dir = config::config_dir()?;
        let cfg =
            config::resolve_managed_whisper_model_config(self.setup.to_config()?, &config_dir);
        config::validate_setup(&cfg)?;
        config::save(&cfg)?;
        self.cfg = Some(cfg);
        self.reload_sessions();
        self.screen = Screen::Sessions;
        Ok(())
    }

    fn reload_sessions(&mut self) {
        if let Some(cfg) = &self.cfg {
            match config::effective_output_dir(cfg).and_then(|dir| audio::list_sessions(&dir)) {
                Ok(sessions) => {
                    self.sessions = sessions;
                    if self.selected_session >= self.sessions.len() {
                        self.selected_session = self.sessions.len().saturating_sub(1);
                    }
                    self.message.clear();
                }
                Err(error) => self.message = error.to_string(),
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

    fn open_selected_session(&mut self) -> Result<()> {
        if let Some(session) = self.sessions.get(self.selected_session) {
            opener::open_folder(&session.path)?;
        }
        Ok(())
    }

    fn open_processing_session(&mut self) -> Result<()> {
        if let Some(path) = &self.processing_session {
            opener::open_folder(path)?;
        }
        Ok(())
    }

    async fn start_recording(&mut self) -> Result<()> {
        let cfg = self.cfg.as_ref().context("Scribe is not configured")?;
        let trimmed_name = self.recording_name.trim();
        let name = if trimmed_name.is_empty() {
            None
        } else {
            Some(trimmed_name)
        };
        let session_dir = audio::create_session_dir(cfg, name)?;
        let recording = Arc::new(AtomicBool::new(true));
        let recording_for_task = recording.clone();
        let sample_rate = cfg.sample_rate;
        let session_for_task = session_dir.clone();

        let task = tokio::task::spawn_blocking(move || {
            if let Err(error) =
                audio::record_loopback(recording_for_task, sample_rate, session_for_task)
            {
                eprintln!("Recording error: {error}");
            }
        });

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
                ProcessingEvent::Complete => {
                    self.processing_step = ProcessingStep::Complete;
                    self.processing_rx = None;
                    self.reload_sessions();
                    self.screen = Screen::Complete;
                }
                ProcessingEvent::Error(message) => {
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
        if let Err(error) = recording.task.await {
            let _ = tx.send(ProcessingEvent::Error(format!(
                "Recording task failed to join: {error}"
            )));
            return;
        }

        let session_dir = recording.session_dir;
        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::Transcribing));
        let wav_path = session_dir.join("recording.wav");
        let transcript = match transcribe::run_whisper(&wav_path, &cfg).await {
            Ok(transcript) => transcript,
            Err(error) => {
                let _ = tx.send(ProcessingEvent::Error(error.to_string()));
                return;
            }
        };

        let txt_path = session_dir.join("transcript.txt");
        if let Err(error) = std::fs::write(&txt_path, &transcript) {
            let _ = tx.send(ProcessingEvent::Error(error.to_string()));
            return;
        }

        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::GeneratingNotes));
        let notes_text = match notes::generate(&transcript, &cfg).await {
            Ok(notes) => notes,
            Err(error) => {
                let _ = tx.send(ProcessingEvent::Error(error.to_string()));
                return;
            }
        };

        let _ = tx.send(ProcessingEvent::Step(ProcessingStep::WritingNotes));
        let full_notes = format!("{notes_text}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
        let notes_path = session_dir.join("notes.md");
        if let Err(error) = std::fs::write(&notes_path, &full_notes) {
            let _ = tx.send(ProcessingEvent::Error(error.to_string()));
            return;
        }

        let _ = tx.send(ProcessingEvent::Complete);
    });
}

impl SetupForm {
    fn from_config(cfg: Option<&config::Config>) -> Self {
        let default_cfg = config::config_dir()
            .map(|dir| config::default_config(&dir))
            .unwrap_or_else(|_| config::Config {
                whisper_bin: Some("whisper-cli".to_string()),
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
    let lines = vec![
        field_line(
            "OpenRouter API key",
            &mask_key(&app.setup.openrouter_api_key),
            app.setup.focus == SetupFocus::ApiKey,
        ),
        field_line(
            "Notes model",
            &app.setup.model,
            app.setup.focus == SetupFocus::NotesModel,
        ),
        Line::from("Whisper backend    whisper.cpp CLI"),
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
}

fn render_sessions(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(45), Constraint::Length(20)])
        .split(area);

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
        chunks[0],
    );

    let output = app
        .cfg
        .as_ref()
        .and_then(|cfg| config::effective_output_dir(cfg).ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let actions = vec![
        Line::from("[ r ] Record"),
        Line::from("[ f ] Refresh"),
        Line::from("[ s ] Settings"),
        Line::from("[ o ] Open"),
        Line::from("[ q ] Quit"),
        Line::from(""),
        Line::from("Enter opens folder"),
        Line::from("Up/Down select"),
        Line::from(""),
        Line::from(format!("Output: {output}")),
        Line::from(app.message.clone()),
    ];
    frame.render_widget(
        Paragraph::new(actions)
            .block(Block::default().title(" Actions ").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        chunks[1],
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
            Line::from("[ Enter ] Start recording    [ Esc ] Cancel"),
        ])
        .block(
            Block::default()
                .title(" New Recording ")
                .borders(Borders::ALL),
        ),
        area,
    );
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

    frame.render_widget(
        Paragraph::new(vec![
            Line::from("* Recording"),
            Line::from(""),
            Line::from(format!("Session      {session}")),
            Line::from(format!("Elapsed      {elapsed}")),
            Line::from(format!("Destination  {destination}")),
            Line::from("Audio file   recording.wav"),
            Line::from(""),
            Line::from("[ Enter ] Stop & Process"),
        ])
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
            Line::from("[ Enter/v ] View    [ b/Esc ] Back to sessions    [ q ] Quit"),
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
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Processing failed"),
            Line::from(""),
            Line::from(app.message.clone()),
            Line::from(""),
            Line::from("[ b/Esc ] Back to sessions    [ q ] Quit"),
        ])
        .block(Block::default().title(" Error ").borders(Borders::ALL))
        .wrap(Wrap { trim: true }),
        area,
    );
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
    fn recording_prompt_cancel_returns_to_sessions() {
        let mut app = App {
            screen: Screen::NewRecording,
            cfg: None,
            setup: SetupForm::from_config(None),
            sessions: Vec::new(),
            selected_session: 0,
            recording_name: "Standup".to_string(),
            recording: None,
            processing_session: None,
            processing_step: ProcessingStep::Finalizing,
            processing_rx: None,
            message: String::new(),
        };

        let quit = tokio_test_handle_key(&mut app, KeyCode::Esc).unwrap();

        assert!(!quit);
        assert_eq!(app.screen, Screen::Sessions);
    }

    #[test]
    fn processing_complete_event_moves_to_complete_screen() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = App {
            screen: Screen::Processing,
            cfg: None,
            setup: SetupForm::from_config(None),
            sessions: Vec::new(),
            selected_session: 0,
            recording_name: String::new(),
            recording: None,
            processing_session: None,
            processing_step: ProcessingStep::Finalizing,
            processing_rx: Some(rx),
            message: String::new(),
        };

        tx.send(ProcessingEvent::Complete).unwrap();
        app.drain_processing_events();

        assert_eq!(app.screen, Screen::Complete);
        assert_eq!(app.processing_step, ProcessingStep::Complete);
        assert!(app.processing_rx.is_none());
    }

    fn tokio_test_handle_key(app: &mut App, code: KeyCode) -> Result<bool> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(handle_key(app, KeyEvent::from(code)))
    }
}
