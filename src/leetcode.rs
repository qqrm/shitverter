use std::{
    cell::OnceCell,
    path::Path,
    sync::{Arc, RwLock},
};

use clokwerk::{Job, Scheduler, TimeUnits};
use teloxide::Bot;
use tokio::{fs::File, io::AsyncWriteExt};

const SUBSCRIBERS: OnceCell<RwLock<Vec<i64>>> = OnceCell::new();

const PATH_TO_SUBS: &str = "/home/data/subscribers_to_daily";

async fn add_subscriber(new_sub: i64) -> Result<(), Box<dyn std::error::Error>> {
    let Some(subs) = SUBSCRIBERS.get() else {
        return Ok(());
    };

    todo!();
    let mut subs = subs.write()?;

    (*subs).push(new_sub);
    write_subscribers_to_file(&subs).await?;
    return Ok(());
}

async fn write_subscribers_to_file(subs: &[i64]) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(PATH_TO_SUBS);
    let mut file = File::create(path).await?;

    for sub in subs {
        file.write_all(format!("{}\n", sub).as_bytes()).await?;
    }

    Ok(())
}

pub async fn process_leetcode_subs(bot: &Bot) -> Result<(), Box<dyn std::error::Error>> {
    let subscribers = Arc::new(RwLock::new(
        read_subscribers()?
            .iter()
            .filter_map(|s| s.parse::<i64>().ok())
            .collect::<Vec<_>>(),
    ));

    // Send leetcode daily to subscribers
    let mut scheduler = Scheduler::new();
    let cloned_bot = bot.clone();
    let cloned_subscribers = subscribers.clone();

    scheduler.every(1.day()).at("10:00").run(move || {
        let subs = cloned_subscribers.read(); // handle this error appropriately
        for &sub in subs.iter() {
            let bot = cloned_bot.clone();
            tokio::spawn(async move {
                send_daily_message(bot, ChatId(sub)).await;
            });
        }
    });

    Ok(())
}

fn read_subscribers() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let path = Path::new(PATH_TO_SUBS);

    let mut subscribers = Vec::new();

    // Open the file or create it if it doesn't exist
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            println!("No subscribers found, creating an empty file.");
            let mut file = File::create(&path)?;
            writeln!(file, "")?; // Write a new line to ensure the file is not empty
            return Ok(subscribers);
        }
    };

    for line in io::BufReader::new(file).lines() {
        if let Ok(ip) = line {
            subscribers.push(ip);
        }
    }

    Ok(subscribers)
}
