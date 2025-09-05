use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::Write,
    path::Path,
};

use anyhow::Context;
use chrono::NaiveDate;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serenity::all::UserId;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct BotState {
    pub used: HashSet<String>,
    pub history: Vec<UsedEntry>,
    pub queue: VecDeque<(UserId, String)>,
}

impl BotState {
    pub fn mark_used(&mut self, date: NaiveDate, word: String, suggested_by: Option<UserId>) {
        self.used.insert(word.clone());
        self.history.push(UsedEntry {
            date,
            word,
            suggested_by,
        });
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UsedEntry {
    pub date: NaiveDate,
    pub word: String,
    pub suggested_by: Option<UserId>,
}

pub struct Store {
    path: String,
    inner: RwLock<BotState>,
}

impl Store {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            inner: RwLock::new(BotState::default()),
        }
    }

    pub fn load(&self) -> anyhow::Result<()> {
        let p = Path::new(&self.path);
        if !p.exists() {
            return Ok(());
        }
        let bytes = fs::read(p).with_context(|| format!("reading {}", self.path))?;
        let state: BotState = serde_json::from_slice(&bytes)?;
        *self.inner.write() = state;
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let state = self.inner.read().clone();
        let buffer = serde_json::to_vec_pretty(&state)?;

        // Write to temp so that if writing causes the failure, it wont have altered the main save
        let tmp = format!("{}.tmp", self.path);
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&buffer)?;
            f.sync_all()?;
        }

        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn with<R>(&self, f: impl FnOnce(&BotState) -> R) -> R {
        f(&self.inner.read())
    }
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut BotState) -> R) -> R {
        let r = f(&mut self.inner.write());
        let _ = self.save();
        r
    }
}
