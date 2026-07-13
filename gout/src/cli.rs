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
    /// 登录远程服务器，保存凭据
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
        /// 后台运行
        #[arg(long, short = 'd')]
        detach: bool,
    },
    /// 创建 UDP 隧道
    Udp {
        /// 本地端口号
        port: u16,
        /// 后台运行
        #[arg(long, short = 'd')]
        detach: bool,
    },
    /// 创建 HTTP 隧道（等价于 TCP）
    Http {
        /// 本地端口号
        port: u16,
        /// 后台运行
        #[arg(long, short = 'd')]
        detach: bool,
    },
    /// 列出本地后台隧道
    List,
    /// 查看后台隧道日志
    Log {
        /// 本地端口号
        port: u16,
        /// 持续跟随（类似 tail -f）
        #[arg(long, short = 'f')]
        follow: bool,
    },
    /// 停止后台隧道
    Kill {
        /// 本地端口号
        port: u16,
    },
}

impl Cli {
    pub fn run() -> anyhow::Result<()> {
        let cli = Cli::parse();

        // 子进程模式（由 -d 启动的 daemon），结束后清理 PID 文件
        if let Some(pidfile) = std::env::var("GOUT_DAEMON_PIDFILE").ok() {
            let result = match cli.command {
                Command::Tcp { port, .. } => crate::cmd_tunnel("tcp", port),
                Command::Udp { port, .. } => crate::cmd_tunnel("udp", port),
                Command::Http { port, .. } => crate::cmd_tunnel("http", port),
                _ => anyhow::bail!("daemon mode unsupported for this command"),
            };
            std::fs::remove_file(&pidfile).ok();
            return result;
        }

        match cli.command {
            Command::Login { server, key } => crate::cmd_login(&server, &key),
            Command::Tcp { port, detach: true } => crate::cmd_start_daemon("tcp", port),
            Command::Tcp { port, detach: false } => crate::cmd_tunnel("tcp", port),
            Command::Udp { port, detach: true } => crate::cmd_start_daemon("udp", port),
            Command::Udp { port, detach: false } => crate::cmd_tunnel("udp", port),
            Command::Http { port, detach: true } => crate::cmd_start_daemon("http", port),
            Command::Http { port, detach: false } => crate::cmd_tunnel("http", port),
            Command::Log { port, follow } => crate::cmd_log(port, follow),
            Command::Kill { port } => crate::cmd_kill(port),
            Command::List => crate::cmd_list(),
        }
    }
}
