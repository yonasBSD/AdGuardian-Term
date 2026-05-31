use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{header::HeaderMap, Client, Response};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct AdGuardFilteringStatus {
  pub filters: Option<Vec<Filter>>,
}

#[derive(Deserialize)]
pub struct Filter {
  pub name: String,
  pub rules_count: u32,
  pub enabled: bool,
}

pub async fn fetch_adguard_filter_list(
  client: &Client,
  endpoint: &str,
  username: &str,
  password: &str,
) -> Result<AdGuardFilteringStatus, anyhow::Error> {
  let url = format!("{}/control/filtering/status", endpoint);

  let auth_string = format!("{}:{}", username, password);
  let auth_header_value = format!("Basic {}", STANDARD.encode(&auth_string));
  let mut headers = HeaderMap::new();
  headers.insert("Authorization", auth_header_value.parse()?);

  let res: Response = client.get(&url).headers(headers).send().await?;
  if !res.status().is_success() {
    return Err(anyhow::anyhow!(
      "Request failed with status code {}",
      res.status()
    ));
  }
  let status: AdGuardFilteringStatus = res.json().await?;

  Ok(status)
}
