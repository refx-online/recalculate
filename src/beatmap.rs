use anyhow::Result;

use std::path::PathBuf;

use tracing::{
    info, 
    error
};

use tokio::fs;

use md5;

pub async fn ensure_osu_file_is_available(
    beatmap_id: i32,
    expected_md5: Option<&str>,
) -> Result<bool> {
    if disk_has_expected_osu_file(beatmap_id, expected_md5).await? {
        return Ok(true);
    }

    info!("Attempting to fetch osu file for {}", beatmap_id);

    match api_get_osu_file(beatmap_id).await {
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

async fn api_get_osu_file(beatmap_id: i32) -> Result<Vec<u8>> {
    // i dont care im using reqwest
    let client = reqwest::Client::new();

    let api_url = std::env::var("BEATMAPS_SERVICE_URL")
        .unwrap_or_else(|_| "balls".to_string());

    let url = format!("{}/v1/get-osu/{}", api_url, beatmap_id);
    let response = client
        .get(&url)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download beatmap: {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}
