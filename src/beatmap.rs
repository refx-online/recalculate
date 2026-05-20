use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, error, warn};
use tokio::fs;
use tokio::sync::Semaphore;
use tokio::time::sleep;

const MAX_CONCURRENT_REQUESTS: usize = 5;
const RETRY_ATTEMPTS: u32 = 3;
const BASE_RETRY_DELAY_MS: u64 = 1000;

lazy_static::lazy_static! {
    static ref RATE_LIMITER: Arc<Semaphore> = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
}

pub async fn ensure_osu_file_is_available(
    beatmap_id: i32,
    expected_md5: Option<&str>,
) -> Result<bool> {
    if beatmap_id >= 1000000000 {
        return Ok(false);
    }
    if disk_has_expected_osu_file(beatmap_id, expected_md5).await? {
        return Ok(true);
    }
    
    info!("Attempting to fetch osu file for {}", beatmap_id);
    
    match api_get_osu_file_with_retry(beatmap_id).await {
        Ok(latest_osu_file) => {
            write_osu_file_to_disk(beatmap_id, &latest_osu_file).await?;
            info!("Successfully fetched osu file for {}", beatmap_id);
            Ok(true)
        }
        Err(e) => {
            error!("Failed to fetch osu file for {}: {}", beatmap_id, e);
            Ok(false)
        }
    }
}

async fn disk_has_expected_osu_file(
    beatmap_id: i32,
    expected_md5: Option<&str>,
) -> Result<bool> {
    let osu_file_path = get_beatmaps_path().join(format!("{}.osu", beatmap_id));
    
    if !fs::try_exists(&osu_file_path).await? {
        return Ok(false);
    }
    
    if let Some(expected_md5) = expected_md5 {
        let file_contents = fs::read(&osu_file_path).await?;
        let digest = md5::compute(&file_contents);
        let file_md5 = format!("{:x}", digest);
        Ok(file_md5.eq_ignore_ascii_case(expected_md5))
    } else {
        Ok(true)
    }
}

async fn write_osu_file_to_disk(beatmap_id: i32, data: &[u8]) -> Result<()> {
    let osu_file_path = get_beatmaps_path().join(format!("{}.osu", beatmap_id));
    
    if let Some(parent) = osu_file_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    
    fs::write(&osu_file_path, data).await?;
    Ok(())
}

fn get_beatmaps_path() -> PathBuf {
    match std::env::var("BEATMAPS_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from("./.data/osu"),
    }
}

async fn api_get_osu_file_with_retry(beatmap_id: i32) -> Result<Vec<u8>> {
    let mut last_error = None;
    
    for attempt in 0..RETRY_ATTEMPTS {
        match api_get_osu_file(beatmap_id).await {
            Ok(data) => return Ok(data),
            Err(e) => {
                last_error = Some(e);
                
                if attempt < RETRY_ATTEMPTS - 1 {
                    let delay = BASE_RETRY_DELAY_MS * 2_u64.pow(attempt);
                    warn!(
                        "Attempt {} failed for beatmap {}, retrying in {}ms",
                        attempt + 1,
                        beatmap_id,
                        delay
                    );
                    sleep(Duration::from_millis(delay)).await;
                }
            }
        }
    }
    
    Err(last_error.unwrap())
}

async fn api_get_osu_file(beatmap_id: i32) -> Result<Vec<u8>> {
    // Acquire semaphore permit to limit concurrent requests
    let _permit = RATE_LIMITER.acquire().await.unwrap();
    
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
        
    let api_url = std::env::var("BEATMAPS_SERVICE_URL")
        .unwrap_or_else(|_| "balls".to_string());
    let url = format!("{}/v1/get-osu/{}", api_url, beatmap_id);
    
    let response = client.get(&url).send().await?;
    
    let status = response.status();
    
    // Handle rate limiting specifically
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Check for Retry-After header
        if let Some(retry_after) = response.headers().get("retry-after") {
            if let Ok(retry_str) = retry_after.to_str() {
                if let Ok(seconds) = retry_str.parse::<u64>() {
                    warn!("Rate limited, waiting {}s before retry", seconds);
                    sleep(Duration::from_secs(seconds)).await;
                }
            }
        }
        return Err(anyhow::anyhow!("Rate limited by API"));
    }
    
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download beatmap: {}",
            status
        ));
    }
    
    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}
