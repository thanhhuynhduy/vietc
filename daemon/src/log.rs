use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    match tracing_journald::layer() {
        Ok(journald) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(journald)
                .try_init()?;
        }
        Err(error) => {
            eprintln!("[vietc] native journald logging unavailable ({error}); using stderr");
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_target(true))
                .try_init()?;
        }
    }

    Ok(())
}

pub fn log_info(msg: &str) {
    tracing::info!(message = msg);
}
