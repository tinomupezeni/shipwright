use anyhow::{Result, Context};
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use futures_util::StreamExt;
use shipwright_common::protocol::{AgentMessage, BuildEvent, RollbackEvent};
use tracing::error;
use std::io::{self, Stdout};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Terminal, Frame,
};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

const SHIPWRIGHT_WS_PORT: u16 = 17671;

struct App {
    project_name: String,
    status: String,
    logs: Vec<String>,
    should_quit: bool,
}

impl App {
    fn new(project_name: String) -> App {
        App {
            project_name,
            status: "Connecting...".to_string(),
            logs: Vec::new(),
            should_quit: false,
        }
    }

    fn add_log(&mut self, log: String) {
        self.logs.push(log);
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
    }
}

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        anyhow::bail!(".shipwright.yml not found. Run 'shipwright init' first.");
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;
    let url = format!("ws://{}:{}", vps.host, SHIPWRIGHT_WS_PORT);

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config.project.name.clone());
    let res = run_app(&mut terminal, &mut app, &url).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {}", err);
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    url: &str,
) -> Result<()> {
    let (ws_stream, _) = match connect_async(url).await {
        Ok(v) => v,
        Err(e) => {
            app.status = format!("Failed to connect: {}", e);
            terminal.draw(|f| ui(f, app))?;
            tokio::time::sleep(Duration::from_secs(3)).await;
            return Err(e).context("Failed to connect to agent");
        }
    };
    
    app.status = "Idle".to_string();
    let (_, mut ws_receiver) = ws_stream.split();

    loop {
        terminal.draw(|f| ui(f, app))?;

        tokio::select! {
            // Handle terminal events
            res = tokio::task::spawn_blocking(|| event::poll(Duration::from_millis(50))) => {
                if let Ok(Ok(true)) = res {
                    if let Event::Key(key) = event::read()? {
                        if let KeyCode::Char('q') = key.code {
                            return Ok(());
                        }
                    }
                }
            }
            // Handle WebSocket messages
            Some(msg) = ws_receiver.next() => {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        let message: AgentMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(_) => continue,
                        };

                        match message {
                            AgentMessage::BuildUpdate { project_name: _, event } => {
                                match event {
                                    BuildEvent::Started => {
                                        app.status = "Deploying...".to_string();
                                        app.add_log("🚀 Deployment started".to_string());
                                    }
                                    BuildEvent::Log(line) => {
                                        app.add_log(format!("  {}", line));
                                    }
                                    BuildEvent::Success => {
                                        app.status = "Healthy".to_string();
                                        app.add_log("✅ Deployment successful!".to_string());
                                    }
                                    BuildEvent::Failed(err) => {
                                        app.status = "Failed".to_string();
                                        app.add_log(format!("❌ Deployment failed: {}", err));
                                    }
                                }
                            }
                            AgentMessage::RollbackUpdate { project_name: _, event } => {
                                match event {
                                    RollbackEvent::SnapshotStarted { .. } => {
                                        app.add_log("📸 Creating deployment snapshot...".to_string());
                                    }
                                    RollbackEvent::RollbackStarted { reason, .. } => {
                                        app.status = "Rolling back...".to_string();
                                        app.add_log(format!("🔄 Rollback initiated: {}", reason));
                                    }
                                    RollbackEvent::RollbackSuccess { .. } => {
                                        app.status = "Healthy (Rolled Back)".to_string();
                                        app.add_log("✅ Rollback successful".to_string());
                                    }
                                    RollbackEvent::RollbackFailed { error } => {
                                        app.status = "Degraded".to_string();
                                        app.add_log(format!("❌ Rollback failed: {}", error));
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                        app.status = "Disconnected".to_string();
                        app.add_log("Connection closed by agent.".to_string());
                        terminal.draw(|f| ui(f, app))?;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        app.status = "Error".to_string();
                        app.add_log(format!("WebSocket error: {}", e));
                        terminal.draw(|f| ui(f, app))?;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.size());

    // Top Bar
    let status_style = match app.status.as_str() {
        "Healthy" | "Healthy (Rolled Back)" => Style::default().fg(Color::Green),
        "Deploying..." | "Rolling back..." => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        "Failed" | "Degraded" | "Error" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::Yellow),
    };

    let header = Paragraph::new(format!(" Project: {} | Status: {}", app.project_name, app.status))
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title(" Shipwright Live Watch ").border_style(status_style));
    f.render_widget(header, chunks[0]);

    // Logs
    let logs: Vec<ListItem> = app
        .logs
        .iter()
        .rev() // Show newest at top or scroll? Let's do newest at bottom but scrollable if possible. 
        // For now, let's just show newest at bottom and take the last N.
        .take(chunks[1].height as usize - 2)
        .map(|content| {
            let style = if content.contains("✅") {
                Style::default().fg(Color::Green)
            } else if content.contains("❌") {
                Style::default().fg(Color::Red)
            } else if content.contains("🚀") || content.contains("🔄") {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(content.as_str()).style(style)
        })
        .collect();

    let log_list = List::new(logs)
        .block(Block::default().borders(Borders::ALL).title(" Deployment Logs "))
        .direction(ratatui::widgets::ListDirection::BottomToTop); // Newest at bottom
    f.render_widget(log_list, chunks[1]);

    // Footer
    let footer = Paragraph::new(" [q] Quit | shipwright.dev ")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    f.render_widget(footer, chunks[2]);
}
