use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "zeron-web", about = "Kratos browser gateway and relay probe")]
struct Cli {
    /// Kratos data directory containing the initialized peer session.
    #[arg(long, default_value_os_t = default_data_dir())]
    data_dir: PathBuf,
    /// External hostname accepted from the HTTPS tunnel.
    #[arg(long, default_value = "dev.embedez.com")]
    hostname: String,
    /// Loopback port exposed to the local tunnel.
    #[arg(long, default_value_t = 3000)]
    port: u16,
}

fn default_data_dir() -> PathBuf {
    if let Some(data_dir) = std::env::var_os("ZERON_DATA_DIR") {
        return PathBuf::from(data_dir);
    }

    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Zeron")
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zeron")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    zeron_web_gateway::serve(cli.data_dir, cli.hostname, cli.port).await
}
