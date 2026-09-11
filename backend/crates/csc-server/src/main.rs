//! csc-server 启动入口：装载资产目录，暴露 REST/WS。
//!
//! 用法：
//! ```text
//! csc-server [assets_dir] [port]
//!   默认: assets_dir = "assets"（当前目录下），port = 8080
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use csc_server::routes::router;
use csc_server::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let assets_dir = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("assets"));
    let port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8080);

    let state = Arc::new(AppState::from_dir(&assets_dir)?);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let standings_count = state.assets.standings.len();
    let top20_years = state.assets.real_top20.years().len();
    println!("csc-server 已启动：http://{addr}");
    println!(
        "  资产：{assets_dir:?}（{standings_count} 期 standings{}{}）",
        if state.assets.baseline.is_some() {
            " + roles_baseline"
        } else {
            ""
        },
        if top20_years > 0 {
            format!(" + {top20_years} 届真实 TOP20")
        } else {
            String::new()
        }
    );
    println!("  REST:  POST /games | GET /games/{{id}}/state | POST /games/{{id}}/advance | ...");
    println!("  WS:    /games/{{id}}/ws（决策面板 + 增量事件流）");

    axum::serve(listener, router().with_state((*state).clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Ctrl+C / SIGTERM 优雅退出。
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\ncsc-server 退出");
}
