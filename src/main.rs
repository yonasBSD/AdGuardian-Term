mod fetch;
mod ui;
mod welcome;
mod widgets;

use reqwest::Client;
use std::{env, time::Duration};
use tokio::time::{interval, MissedTickBehavior};

use ui::draw_ui;

use fetch::{
  fetch_filters::{fetch_adguard_filter_list, AdGuardFilteringStatus},
  fetch_query_log::{fetch_adguard_query_log, Query},
  fetch_stats::{fetch_adguard_stats, StatsResponse},
  fetch_status::{fetch_adguard_status, StatusResponse},
};

/// Fetch the query log, stats and status together, so a failure leaves the UI's
/// data in sync (all-or-nothing) rather than partially updated.
async fn fetch_all(
  client: &Client,
  hostname: &str,
  username: &str,
  password: &str,
  query_log_limit: u32,
) -> anyhow::Result<(Vec<Query>, StatsResponse, StatusResponse)> {
  let queries =
    fetch_adguard_query_log(client, hostname, username, password, query_log_limit).await?;
  let stats = fetch_adguard_stats(client, hostname, username, password).await?;
  let status = fetch_adguard_status(client, hostname, username, password).await?;
  Ok((queries.data, stats, status))
}

async fn run() -> anyhow::Result<()> {
  // Per-request timeout (seconds), clamped to at least 1, so no request can hang
  let timeout_secs: u64 = env::var("ADGUARD_TIMEOUT")
    .unwrap_or_else(|_| "5".into())
    .parse::<u64>()?
    .max(1);
  let client = Client::builder()
    .timeout(Duration::from_secs(timeout_secs))
    .build()?;

  // AdGuard instance details, from env vars (verified in welcome.rs)
  let ip = env::var("ADGUARD_IP")?;
  let port = env::var("ADGUARD_PORT")?;
  let protocol = env::var("ADGUARD_PROTOCOL").unwrap_or("http".to_string());
  let hostname = format!("{}://{}:{}", protocol, ip, port);
  let username = env::var("ADGUARD_USERNAME")?;
  let password = env::var("ADGUARD_PASSWORD")?;

  // Fetch the filter list, use empty list on failures is fine
  let filters = welcome::with_retries(
    3,
    Duration::from_secs(5),
    "Fetching AdGuard filters",
    || fetch_adguard_filter_list(&client, &hostname, &username, &password),
  )
  .await
  .unwrap_or_else(|e| {
    eprintln!("Could not fetch filter list, starting without it: {}", e);
    AdGuardFilteringStatus { filters: None }
  });

  // Open channels for data fetching where updates are required
  let (queries_tx, queries_rx) = tokio::sync::mpsc::channel(1);
  let (stats_tx, stats_rx) = tokio::sync::mpsc::channel(1);
  let (status_tx, status_rx) = tokio::sync::mpsc::channel(1);

  // Shutdown signal, set by the UI when the user quits
  let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

  // Spawn the UI task, pass data and update channels
  let draw_ui_task = tokio::spawn(draw_ui(
    queries_rx,
    stats_rx,
    status_rx,
    filters,
    shutdown_tx,
  ));

  // Get update interval (in seconds), clamped to at least 1 (interval() panics on zero)
  let interval_secs: u64 = env::var("ADGUARD_UPDATE_INTERVAL")
    .unwrap_or_else(|_| "2".into())
    .parse::<u64>()?
    .max(1);
  let mut interval = interval(Duration::from_secs(interval_secs));
  interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

  // Max num of query log entries to fetch per update
  let query_log_limit: u32 = env::var("ADGUARD_QUERYLOG_LIMIT")
    .unwrap_or_else(|_| "100".into())
    .parse()?;

  // Open loop for fetching data at the specified interval
  loop {
    tokio::select! {
        _ = interval.tick() => {
            // Check data is ok, just skip this update on transient error
            if let Ok((queries, stats, status)) =
                fetch_all(&client, &hostname, &username, &password, query_log_limit).await
            {
                // A send error means the UI has shut down, so stop fetching
                if queries_tx.send(queries).await.is_err()
                    || stats_tx.send(stats).await.is_err()
                    || status_tx.send(status).await.is_err()
                {
                    break;
                }
            }
        }
        // Resolves when the UI sets the shutdown flag, or drops the sender
        _ = shutdown_rx.changed() => {
            break;
        }
    }
  }

  draw_ui_task.await??;

  Ok(())
}

fn main() {
  let rt = tokio::runtime::Runtime::new().expect("failed to start async runtime");
  rt.block_on(async {
    welcome::welcome().await.unwrap_or_else(|e| {
      eprintln!("Failed to initialize: {}", e);
      std::process::exit(1);
    });

    run()
      .await
      .map_err(|e| {
        eprintln!("Failed to run: {}", e);
        std::io::Error::other(format!("Failed to run: {}", e))
      })
      .unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
      });
  });
}
