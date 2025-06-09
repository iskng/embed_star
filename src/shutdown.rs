use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use std::time::Duration;

#[derive(Clone)]
pub struct ShutdownController {
    tx: broadcast::Sender<()>,
}

impl ShutdownController {
    pub fn new() -> (Self, ShutdownReceiver) {
        let (tx, rx) = broadcast::channel(1);
        (
            Self { tx },
            ShutdownReceiver { rx }
        )
    }
    
    pub fn shutdown(&self) {
        let _ = self.tx.send(());
    }
}

pub struct ShutdownReceiver {
    rx: broadcast::Receiver<()>,
}

impl ShutdownReceiver {
    pub async fn wait_for_shutdown(mut self) {
        let _ = self.rx.recv().await;
    }
    
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.rx.resubscribe()
    }
}

pub struct GracefulShutdown {
    tasks: Vec<(String, JoinHandle<()>)>,
    controller: ShutdownController,
}

impl GracefulShutdown {
    pub fn new(controller: ShutdownController) -> Self {
        Self {
            tasks: Vec::new(),
            controller,
        }
    }
    
    pub fn register_task(&mut self, name: String, handle: JoinHandle<()>) {
        self.tasks.push((name, handle));
    }
    
    pub async fn shutdown(self, timeout: Duration) {
        info!("Initiating graceful shutdown...");
        
        // Signal all tasks to shutdown
        self.controller.shutdown();
        
        // Wait for all tasks with timeout
        let shutdown_future = async {
            for (name, handle) in self.tasks {
                match handle.await {
                    Ok(_) => info!("Task '{}' shut down successfully", name),
                    Err(e) => error!("Task '{}' panicked during shutdown: {:?}", name, e),
                }
            }
        };
        
        match tokio::time::timeout(timeout, shutdown_future).await {
            Ok(_) => info!("All tasks shut down successfully"),
            Err(_) => warn!("Shutdown timeout exceeded, some tasks may not have completed cleanly"),
        }
    }
}

pub async fn setup_signal_handlers() -> ShutdownReceiver {
    let (controller, receiver) = ShutdownController::new();
    
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();
        
        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };
        
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();
        
        tokio::select! {
            _ = ctrl_c => {
                info!("Received SIGINT (Ctrl+C)");
            }
            _ = terminate => {
                info!("Received SIGTERM");
            }
        }
        
        controller.shutdown();
    });
    
    receiver
}