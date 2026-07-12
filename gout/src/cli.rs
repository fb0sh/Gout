/// CLI 命令解析

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "gout", version, about = "轻量内网穿透工具")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 登录远程服务器，保存凭据到 ~/.goutrc
    Login {
        /// 服务器地址，如 `server.example.com:8080`
        server: String,
        /// API key
        key: String,
    },
    /// 创建 TCP 隧道
    Tcp {
        /// 本地端口号
        port: u16,
    },
    /// 创建 UDP 隧道
    Udp {
        /// 本地端口号
        port: u16,
    },
    /// 创建 HTTP 隧道（v0.1 等价于 TCP）
    Http {
        /// 本地端口号
        port: u16,
    },
    /// 列出活跃隧道
    List,
}

impl Cli {
    pub fn run() -> anyhow::Result<()> {
        let cli = Cli::parse();
        match cli.command {
            Command::Login { server, key } => crate::cmd_login(&server, &key),
            Command::Tcp { port } => crate::cmd_tunnel("tcp", port),
            Command::Udp { port } => crate::cmd_tunnel("udp", port),
            Command::Http { port } => crate::cmd_tunnel("http", port),
            Command::List => crate::cmd_list(),
        }
    }
}
