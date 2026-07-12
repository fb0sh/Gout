/// 服务端配置，通过 CLI 参数传入。

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "goutd", version, about = "Gout 服务端守护进程")]
pub struct ServerConfig {
    /// HTTP / Web 面板监听地址
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub http_addr: String,

    /// 数据通道监听地址
    #[arg(long, default_value = "0.0.0.0:8081")]
    pub data_addr: String,

    /// 公网端口范围起始
    #[arg(long, default_value = "10000")]
    pub port_start: u16,

    /// 公网端口范围结束
    #[arg(long, default_value = "10100")]
    pub port_end: u16,

    /// 数据存储目录（keys.toml 存放位置）
    #[arg(long, default_value = "./data")]
    pub data_dir: PathBuf,
}
