use anyhow::Result;
use clap::Parser;
use futures::future::join_all;

use redis::AsyncCommands;
use serde_json::Value;

use sqlx::{Row, Pool, MySql};
use std::path::Path;

use tracing::{info, warn, debug};

mod beatmap;
mod context;
mod game_mode;

use beatmap::ensure_osu_file_is_available;
use context::Context;

use game_mode::{
    GameMode, 
    parse_mods, 
    GameMods
};

use refx_pp::Beatmap;

const CHUNK_SIZE: usize = 100;

const UNRESTRICTED: i32 = 1 << 0;

#[derive(Parser, Debug)]
pub struct Args {
    /// enable debug logging
    #[arg(short, long, default_value_t = false)]
    debug: bool,

    /// disable recalculating scores
    #[arg(long, default_value_t = false)]
    no_scores: bool,

    /// disable recalculating user stats
    #[arg(long, default_value_t = false)]
    no_stats: bool,

    /// game modes to process
    #[arg(
        short, 
        long, 
        value_delimiter = ',',
        default_values = ["0", "1", "2", "3", "4", "5", "6", "8", "12", "16", "20"]
    )]
    mode: Vec<u8>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct Score {
    id: u64,
    mode: i8,
    mods: i32,
    map_md5: String,
    pp: f32,

    #[allow(dead_code)]
    acc: f32,

    max_combo: i32,
    ngeki: i32,
    n300: i32,
    nkatu: i32,
    n100: i32,
    n50: i32,
    nmiss: i32,

    #[allow(dead_code)]
    userid: i32,

    map_id: i32,

    mods_json: Option<sqlx::types::Json<serde_json::Value>>,
    lazer: bool,
    clock_rate: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct BestScore {
    pp: f32,
    acc: f32,
}

#[derive(Debug, sqlx::FromRow)]  
struct UserInfo {
    country: String,
    privs: i32,
}

fn divide_chunks<T: Clone>(values: &[T], chunk_size: usize) -> Vec<Vec<T>> {
    values.chunks(chunk_size).map(|chunk| chunk.to_vec()).collect()
}

async fn recalculate_score(
    score: &Score,
    beatmap_path: &Path,
    ctx: &Context,
) -> Result<()> {
    let beatmap = match ctx.get_beatmap(score.map_id).await {
        Some(bm) => bm,
        None => {
            let bm = Beatmap::from_path(beatmap_path)?;
            ctx.cache_beatmap(score.map_id, bm.clone()).await;
            bm
        }
    };

    let mut calculator = beatmap
        .performance()
        .combo(score.max_combo as u32)
        .n300(score.n300 as u32)
        .n100(score.n100 as u32)
        .n50(score.n50 as u32)
        .misses(score.nmiss as u32)
        .n_geki(score.ngeki as u32)
        .n_katu(score.nkatu as u32)
        .lazer(score.lazer);

    let game_mode = GameMode::try_from(score.mode as u8).unwrap_or(GameMode::Osu);

    let mods = if score.lazer {
        if let Some(mods_json) = &score.mods_json {
            // might wanna just use the mods_json directly instead of converting back to string
            // but whatever
            mods_json.0.clone()
        } else {
            Value::from(score.mods)
        }
    } else {
        Value::from(score.mods)
    };

    let mods_str = match &mods {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Object(_) => serde_json::to_string(&mods).unwrap_or_default(),
        _ => String::new(),
    };

    if !mods_str.is_empty() {
        match parse_mods(&mods_str, game_mode) {
            Ok(parsed_mods) => {
                calculator = match parsed_mods {
                    GameMods::Legacy(legacy_mods) => {
                        if score.lazer {
                            calculator.mods(legacy_mods)
                        } else {
                            calculator.mods(legacy_mods.bits())
                        }
                    },
                    GameMods::Intermode(intermode_mods) => calculator.mods(intermode_mods),
                    GameMods::Lazer(lazer_mods) => calculator.mods(lazer_mods),
                };
            }
            Err(e) => {
                warn!("Failed to parse mods '{}' for score {}: {}", mods_str, score.id, e);
            }
        }
    }

    if let Some(clock_rate) = score.clock_rate {
        if clock_rate != -1.0 {
            calculator = calculator.clock_rate(clock_rate);
        }
    }

    let new_pp = calculator.calculate().pp();
    
    // this happens
    let new_pp = if new_pp.is_nan() || new_pp.is_infinite() {
        0.0
    } else {
        new_pp
    };

    match sqlx::query("UPDATE scores SET pp = ? WHERE id = ?")
        .bind(new_pp)
        .bind(score.id)
        .execute(&ctx.database)
        .await
    {
        Ok(_) => {
            info!("Recalculated score ID {} ({:.3}pp -> {:.3}pp)", 
                  score.id, score.pp, new_pp);
        }
        Err(e) => {
            warn!("Failed to update score ID {}: {}", score.id, e);

            sqlx::query("UPDATE scores SET pp = ? WHERE id = ?")
                .bind(0.0)
                .bind(score.id)
                .execute(&ctx.database)
                .await?;
        }
    }

    Ok(())
}

async fn process_score_chunk(
    chunk: &[Score],
    ctx: &Context,
) -> Result<()> {
    let beatmaps_path = ctx.beatmaps_path();
    
    let tasks: Vec<_> = chunk.iter().map(|score| async move {
        let osu_file_path = beatmaps_path.join(format!("{}.osu", score.map_id));
        
        if ensure_osu_file_is_available(score.map_id, Some(&score.map_md5)).await? {
            recalculate_score(score, &osu_file_path, ctx).await
        } else {
            Ok(())
        }
    }).collect();

    let results = join_all(tasks).await;
    
    for result in results {
        if let Err(e) = result {
            warn!("Error processing score: {}", e);
        }
    }

    Ok(())
}

async fn recalculate_user(
    user_id: i32,
    game_mode: GameMode,
    ctx: &Context,
) -> Result<()> {
    let best_scores: Vec<BestScore> = sqlx::query_as(
        "SELECT s.pp, s.acc FROM scores s 
        INNER JOIN maps m ON s.map_md5 = m.md5 
        WHERE s.userid = ? AND s.mode = ? 
        AND s.status = 2 AND m.status IN (2, 3) 
        ORDER BY s.pp DESC"
    )
        .bind(user_id)
        .bind(game_mode as u8)
        .fetch_all(&ctx.database)
        .await?;

    let total_scores = best_scores.len();

    if total_scores == 0 {
        return Ok(());
    }

    let weighted_acc: f32 = best_scores.iter().enumerate()
        .map(|(i, score)| score.acc * 0.95_f32.powi(i as i32))
        .sum();
    
    let bonus_acc = 100.0 / (20.0 * (1.0 - 0.95_f32.powi(total_scores as i32)));
    let acc = (weighted_acc * bonus_acc) / 100.0;

    let weighted_pp: f32 = best_scores.iter().enumerate()
        .map(|(i, score)| score.pp * 0.95_f32.powi(i as i32))
        .sum();
    
    let bonus_pp: f32 = 416.6667 * (1.0 - 0.9994_f32.powi(total_scores as i32));
    let pp = (weighted_pp + bonus_pp).round();

    sqlx::query("UPDATE stats SET pp = ?, acc = ? WHERE id = ? AND mode = ?")
        .bind(pp)
        .bind(acc)
        .bind(user_id)
        .bind(game_mode as u8)
        .execute(&ctx.database)
        .await?;

    let user_info: UserInfo = sqlx::query_as("SELECT country, priv as privs FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&ctx.database)
        .await?;

    if (user_info.privs & UNRESTRICTED) != 0 {
        let mut redis_conn = ctx.redis.get_async_connection().await?;
        
        let _: () = redis_conn.zadd(
            format!("bancho:leaderboard:{}", game_mode as u8),
            user_id,
            pp as i32
        ).await?;

        let _: () = redis_conn.zadd(
            format!("bancho:leaderboard:{}:{}", game_mode as u8, user_info.country),
            user_id,
            pp as i32
        ).await?;
    }

    debug!("Recalculated user ID {} ({:.3}pp, {:.3}%)", user_id, pp, acc);

    Ok(())
}

async fn process_user_chunk(
    chunk: &[i32],
    game_mode: GameMode,
    ctx: &Context,
) -> Result<()> {
    let tasks: Vec<_> = chunk.iter().map(|&user_id| {
        recalculate_user(user_id, game_mode, ctx)
    }).collect();

    let results = join_all(tasks).await;
    
    for result in results {
        if let Err(e) = result {
            warn!("Error processing user: {}", e);
        }
    }

    Ok(())
}

async fn recalculate_mode_users(mode: GameMode, ctx: &Context) -> Result<()> {
    let user_ids: Vec<i32> = sqlx::query("SELECT id FROM users")
        .fetch_all(&ctx.database)
        .await?
        .iter()
        .map(|row| row.get::<i32, _>("id"))
        .collect();

    let chunks = divide_chunks(&user_ids, CHUNK_SIZE);
    
    for chunk in chunks {
        process_user_chunk(&chunk, mode, ctx).await?;
    }

    Ok(())
}

async fn recalculate_score_statuses(mode: GameMode, ctx: &Context) -> Result<()> {
    // For each (userid, map_md5) group, set the best score (highest pp, tiebreak by score) to
    // status=2, rest to status=1
    sqlx::query(
        r#"
        UPDATE scores s
        INNER JOIN (
            SELECT userid, map_md5,
                   SUBSTRING_INDEX(GROUP_CONCAT(id ORDER BY pp DESC, score DESC), ',', 1) AS best_id
            FROM scores
            WHERE mode = ? AND status IN (1, 2)
            GROUP BY userid, map_md5
        ) best ON s.userid = best.userid AND s.map_md5 = best.map_md5
        SET s.status = CASE WHEN s.id = best.best_id THEN 2 ELSE 1 END
        WHERE s.mode = ? AND s.status IN (1, 2)
        "#
    )
        .bind(mode as u8)
        .bind(mode as u8)
        .execute(&ctx.database)
        .await?;

    info!("Updated score statuses for mode {:?}", mode);
    Ok(())
}

async fn recalculate_mode_scores(mode: GameMode, ctx: &Context) -> Result<()> {
    // we dont talk about lazer check
    let scores: Vec<Score> = sqlx::query_as(
        r#"
        SELECT 
            scores.score, scores.id, scores.mode, scores.mods, scores.map_md5,
            scores.pp, scores.acc, scores.max_combo,
            scores.ngeki, scores.n300, scores.nkatu, scores.n100, scores.n50, scores.nmiss,
            scores.userid, scores.clock_rate, 
            maps.id AS map_id,
            lazer_scores.mods_json,
            CASE 
                WHEN lazer_scores.score_id IS NOT NULL THEN TRUE
                ELSE FALSE
            END AS lazer
        FROM scores
        INNER JOIN maps ON scores.map_md5 = maps.md5
        LEFT JOIN lazer_scores ON lazer_scores.score_id = scores.id
        WHERE scores.status IN (1, 2)
          AND scores.mode = ?
        ORDER BY scores.pp DESC
        "#
    )
        .bind(mode as u8)
        .fetch_all(&ctx.database)
        .await?;

    let chunks = divide_chunks(&scores, CHUNK_SIZE);

    for chunk in chunks {
        process_score_chunk(&chunk, ctx).await?;
    }

    recalculate_score_statuses(mode, ctx).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    
    let args = Args::parse();

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(if args.debug { "debug" } else { "info" })
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "balls".to_string());
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "balls".to_string());

    let database: Pool<MySql> = sqlx::mysql::MySqlPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;

    let redis_client = redis::Client::open(redis_url)?;

    let ctx = Context::new(database, redis_client).await?;

    for mode_num in args.mode {
        if let Ok(mode) = GameMode::try_from(mode_num) {
            info!("Processing mode: {:?}", mode);
            
            if !args.no_scores {
                info!("Recalculating scores for mode {:?}", mode);
                recalculate_mode_scores(mode, &ctx).await?;
            }

            if !args.no_stats {
                info!("Recalculating user stats for mode {:?}", mode);
                recalculate_mode_users(mode, &ctx).await?;
            }
        } else {
            warn!("Unknown game mode: {}", mode_num);
        }
    }

    info!("Recalculation completed.");

    Ok(())
}
