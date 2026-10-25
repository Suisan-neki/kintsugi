//! 子プロセスの強制終了に代わり、解析の反復で同じ期限を確認する。
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    time::{Duration, Instant},
};
#[derive(Debug)]
pub(super) struct Budget {
    pub until: Instant,
    stopped: Cell<bool>,
    patterns: RefCell<HashMap<String, regex::Regex>>,
}
impl Budget {
    pub fn new() -> Self {
        Self {
            until: Instant::now() + Duration::from_secs(30),
            stopped: Cell::new(false),
            patterns: RefCell::new(HashMap::new()),
        }
    }
    /// 同じ解析内で生成した名前入りパターンを共用する。上限は256種類。
    pub fn pattern(&self, source: &str) -> regex::Regex {
        let mut patterns = self.patterns.borrow_mut();
        if let Some(pattern) = patterns.get(source) {
            return pattern.clone();
        }
        if patterns.len() >= 256 {
            patterns.clear();
        }
        let pattern = regex::Regex::new(source).expect("名前をエスケープした内部パターン");
        patterns.insert(source.to_owned(), pattern.clone());
        pattern
    }
    pub fn check(&self) -> bool {
        if Instant::now() >= self.until {
            self.stopped.set(true);
        }
        !self.stopped.get()
    }
    pub fn stop(&self) {
        self.stopped.set(true);
    }
}
