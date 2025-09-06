use anyhow::Result;

use sqlx::{
    Pool, 
    MySql
};

use std::collections::HashMap;

use std::path::{
    Path, 
    PathBuf
};

use tokio::sync::RwLock;

use refx_pp::Beatmap;

pub struct Context {
    pub database: Pool<MySql>,
    pub redis: redis::Client,

    beatmaps: RwLock<HashMap<i32, Beatmap>>,
    beatmaps_path: PathBuf,
}

impl Context {
    pub async fn new(database: Pool<MySql>, redis: redis::Client) -> Result<Self> {
        let beatmaps_path = std::env::var("BEATMAPS_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(".data")
                    .join("osu")
            });

        tokio::fs::create_dir_all(&beatmaps_path).await?;

        Ok(Self {
            database,
            redis,
            beatmaps: RwLock::new(HashMap::new()),
            beatmaps_path,
        })
    }

    pub async fn get_beatmap(&self, map_id: i32) -> Option<Beatmap> {
        let beatmaps = self.beatmaps.read().await;
        beatmaps.get(&map_id).cloned()
    }

    pub async fn cache_beatmap(&self, map_id: i32, beatmap: Beatmap) {
        let mut beatmaps = self.beatmaps.write().await;
        beatmaps.insert(map_id, beatmap);
    }

    pub fn beatmaps_path(&self) -> &Path {
        &self.beatmaps_path
    }
}
