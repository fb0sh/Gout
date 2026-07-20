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
    /// 登录远程服务器（等价于 `server set`）
    Login {
        /// 服务器名称（可选，默认按地址自动生成）
        name: Option<String>,
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
        /// 服务器名称或地址（默认使用配置中的 default_server）
        #[arg(long, short = 's')]
        server: Option<String>,
        /// 远端公网端口（可选，不指定由服务端自动分配）
        #[arg(long = "remote-port", short = 'r')]
        remote_port: Option<u16>,
    },
    /// 创建 UDP 隧道
    Udp {
        /// 本地端口号
        port: u16,
        /// 后台运行
        #[arg(long, short = 'd')]
        detach: bool,
        /// 服务器名称或地址
        #[arg(long, short = 's')]
        server: Option<String>,
        /// 远端公网端口（可选，不指定由服务端自动分配）
        #[arg(long = "remote-port", short = 'r')]
        remote_port: Option<u16>,
    },
    /// 创建 HTTP 隧道（等价于 TCP）
    Http {
        /// 本地端口号
        port: u16,
        /// 后台运行
        #[arg(long, short = 'd')]
        detach: bool,
        /// 服务器名称或地址
        #[arg(long, short = 's')]
        server: Option<String>,
        /// 远端公网端口（可选，不指定由服务端自动分配）
        #[arg(long = "remote-port", short = 'r')]
        remote_port: Option<u16>,
    },
    /// 列出本地后台隧道（按 server 分组）
    #[clap(alias = "ls")]
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
    /// 管理 server
    #[command(subcommand)]
    Server(ServerCmd),
}

#[derive(Subcommand, Debug)]
pub enum ServerCmd {
    /// 添加或更新 server
    Set {
        /// server 名称
        name: String,
        /// server 地址，如 `server.example.com:8080`
        host: String,
        /// API key
        key: String,
    },
    /// 设置默认 server
    Default {
        /// server 名称
        name: String,
    },
    /// 删除 server
    Unset {
        /// server 名称
        name: String,
    },
    /// 显示所有 server
    Show,
}

impl Cli {
    pub fn run() -> anyhow::Result<()> {
        let cli = Cli::parse();

        // 子进程模式（由 -d 启动的 daemon），结束后清理 PID 文件
        if let Some(pidfile) = std::env::var("GOUT_DAEMON_PIDFILE").ok() {
            let result = match cli.command {
                Command::Tcp { port, .. } => crate::cmd_tunnel("tcp", port, None, None),
                Command::Udp { port, .. } => crate::cmd_tunnel("udp", port, None, None),
                Command::Http { port, .. } => crate::cmd_tunnel("http", port, None, None),
                _ => anyhow::bail!("daemon mode unsupported for this command"),
            };
            std::fs::remove_file(&pidfile).ok();
            return result;
        }

        match cli.command {
            Command::Login { name, server, key } => {
                let n = name.unwrap_or_else(|| {
                    server.split(':').next().unwrap_or(&server).to_string()
                });
                crate::cmd_server_set(&n, &server, &key)
            }
            Command::Tcp { port, detach: true, server, remote_port } => crate::cmd_start_daemon("tcp", port, server.as_deref(), remote_port),
            Command::Tcp { port, detach: false, server, remote_port } => crate::cmd_tunnel("tcp", port, server.as_deref(), remote_port),
            Command::Udp { port, detach: true, server, remote_port } => crate::cmd_start_daemon("udp", port, server.as_deref(), remote_port),
            Command::Udp { port, detach: false, server, remote_port } => crate::cmd_tunnel("udp", port, server.as_deref(), remote_port),
            Command::Http { port, detach: true, server, remote_port } => crate::cmd_start_daemon("http", port, server.as_deref(), remote_port),
            Command::Http { port, detach: false, server, remote_port } => crate::cmd_tunnel("http", port, server.as_deref(), remote_port),
            Command::Log { port, follow } => crate::cmd_log(port, follow),
            Command::Kill { port } => crate::cmd_kill(port),
            Command::List => crate::cmd_list(),
            Command::Server(cmd) => match cmd {
                ServerCmd::Set { name, host, key } => crate::cmd_server_set(&name, &host, &key),
                ServerCmd::Default { name } => crate::cmd_server_default(&name),
                ServerCmd::Unset { name } => crate::cmd_server_unset(&name),
                ServerCmd::Show => crate::cmd_server_show(),
            },
        }
    }
}
