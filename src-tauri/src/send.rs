//! Sending to several boards at once: a single board, a group, or all of them.
//!
//! Targets are strings: "this" (this computer's board), a saved board's id, "group:<id>" or
//! "all" (this board and every saved one). Each board gets its own copy of the message,
//! laid out for its size, in parallel and with its own timeout, so one slow or failing board
//! never holds up the others.

use crate::boards::SavedBoard;
use crate::vestaboard::{self, Grid, Transition};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::future::Future;
use std::path::Path;
use std::time::Duration;

pub const HERE: &str = "this";
pub const ALL: &str = "all";
pub const GROUP_PREFIX: &str = "group:";
/// How long one board may take to answer before it's reported as failed.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// A named set of boards. Members are "this" or saved boards' ids, never keys.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
}

pub fn load_groups(path: &Path) -> Vec<Group> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_groups(path: &Path, groups: &[Group]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(groups) {
        let _ = std::fs::write(path, s);
    }
}

/// Tidies groups from the UI: names trimmed, members limited to boards that exist, each once.
pub fn clean_groups(groups: Vec<Group>, boards: &[SavedBoard]) -> Vec<Group> {
    groups
        .into_iter()
        .filter(|g| !g.id.is_empty())
        .map(|g| {
            let mut members: Vec<String> = vec![];
            for m in g.members {
                if (m == HERE || boards.iter().any(|b| b.id == m)) && !members.contains(&m) {
                    members.push(m);
                }
            }
            let name = g.name.trim();
            Group { name: if name.is_empty() { "Group".into() } else { name.into() }, members, id: g.id }
        })
        .collect()
}

/// The boards a target stands for: "this" and/or saved boards' ids, each once.
pub fn destinations(target: &str, boards: &[SavedBoard], groups: &[Group]) -> Result<Vec<String>, String> {
    let ids: Vec<String> = if target == ALL {
        std::iter::once(HERE.to_string()).chain(boards.iter().map(|b| b.id.clone())).collect()
    } else if let Some(gid) = target.strip_prefix(GROUP_PREFIX) {
        let g = groups.iter().find(|g| g.id == gid).ok_or("That group was removed")?;
        g.members.clone()
    } else if target == HERE || boards.iter().any(|b| b.id == target) {
        vec![target.to_string()]
    } else {
        return Err("That board was removed".into());
    };
    let mut out: Vec<String> = vec![];
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() {
        return Err("That group has no boards in it yet".into());
    }
    Ok(out)
}

/// Runs `deliver` for every item at once, each with `limit` to finish, and reports every
/// outcome in the items' order. A failure or timeout affects only its own item.
pub async fn fan_out<T, F, Fut>(items: Vec<T>, limit: Duration, deliver: F) -> Vec<(T, Result<(), String>)>
where
    T: Clone + Send + 'static,
    F: Fn(T) -> Fut,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let mut set = tokio::task::JoinSet::new();
    for (i, item) in items.iter().cloned().enumerate() {
        let job = deliver(item);
        set.spawn(async move {
            let r = match tokio::time::timeout(limit, job).await {
                Ok(r) => r,
                Err(_) => Err("The board didn't answer in time".to_string()),
            };
            (i, r)
        });
    }
    let mut results: Vec<Option<Result<(), String>>> = vec![None; items.len()];
    while let Some(done) = set.join_next().await {
        if let Ok((i, r)) = done {
            results[i] = Some(r);
        }
    }
    items
        .into_iter()
        .zip(results)
        .map(|(item, r)| (item, r.unwrap_or_else(|| Err("Sending stopped unexpectedly".into()))))
        .collect()
}

/// One message to a saved board. Rattleboards and real boards both take it on the Local API.
pub async fn post(c: &reqwest::Client, b: &SavedBoard, key: &str, layout: &Grid, tr: &Transition) -> Result<(), String> {
    if key.is_empty() {
        return Err("No key saved for this board".into());
    }
    if b.kind == "cloud" {
        return vestaboard::write_cloud(c, key, layout).await;
    }
    let mut body = serde_json::Map::new();
    body.insert("characters".into(), json!(layout));
    tr.add_to(&mut body);
    vestaboard::write_local(c, &b.host, key, &Value::Object(body)).await
}

/// How Compose shows a failed delivery.
pub fn status_of(err: &str) -> &'static str {
    if err.contains("rate-limiting") {
        "rate-limited"
    } else {
        "failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server;
    use std::sync::{Arc, Mutex};

    fn board(id: &str, rows: usize, cols: usize) -> SavedBoard {
        SavedBoard { id: id.into(), name: id.to_uppercase(), kind: "rattleboard".into(), host: String::new(), rows, cols, adaptive: false }
    }

    #[test]
    fn targets() {
        let boards = [board("a", 6, 22), board("b", 3, 15)];
        let groups = [
            Group { id: "g1".into(), name: "Downstairs".into(), members: vec!["b".into(), "this".into(), "b".into()] },
            Group { id: "g2".into(), name: "Empty".into(), members: vec![] },
        ];
        assert_eq!(destinations("all", &boards, &groups).unwrap(), ["this", "a", "b"]);
        assert_eq!(destinations("group:g1", &boards, &groups).unwrap(), ["b", "this"]);
        assert_eq!(destinations("a", &boards, &groups).unwrap(), ["a"]);
        assert_eq!(destinations("this", &[], &[]).unwrap(), ["this"]);
        assert!(destinations("group:g2", &boards, &groups).is_err());
        assert!(destinations("group:nope", &boards, &groups).is_err());
        assert!(destinations("gone", &boards, &groups).is_err());

        let messy = vec![Group { id: "g".into(), name: "  ".into(), members: vec!["a".into(), "gone".into(), "a".into(), "this".into()] }];
        let clean = clean_groups(messy, &boards);
        assert_eq!((clean[0].name.as_str(), clean[0].members.clone()), ("Group", vec!["a".to_string(), "this".to_string()]));
        // a board can be in several groups
        let two = clean_groups(vec![Group { id: "x".into(), name: "X".into(), members: vec!["a".into()] }, Group { id: "y".into(), name: "Y".into(), members: vec!["a".into()] }], &boards);
        assert_eq!(two.len(), 2);
    }

    /// Each board's copy is laid out for it: text at its size (or an adaptive board's hint), a
    /// grid centred on a fixed board and kept as drawn for an adaptive one.
    #[test]
    fn per_board_layout() {
        let text = json!({"text": "Good morning everyone", "strategy": "row", "rows": 8, "cols": 30});
        let (big, tr) = server::message(&text, (6, 22), false).unwrap();
        let (small, _) = server::message(&text, (3, 15), false).unwrap();
        let (hinted, _) = server::message(&text, (6, 22), true).unwrap();
        assert_eq!(big, vestaboard::text_layout("Good morning everyone", 6, 22));
        assert_eq!(small, vestaboard::text_layout("Good morning everyone", 3, 15));
        assert_eq!(hinted, vestaboard::text_layout("Good morning everyone", 8, 30));
        assert_eq!(tr.strategy, "row");

        let grid = json!({"characters": vec![vec![1u8; 15]; 3], "strategy": "column"});
        let (kept, tr) = server::message(&grid, (6, 22), true).unwrap();
        assert_eq!((kept.len(), kept[0].len(), tr.strategy.as_str()), (3, 15, "column"));
        let (centred, _) = server::message(&grid, (6, 22), false).unwrap();
        assert_eq!((centred.len(), centred[1][3], centred[0][3]), (6, 1, 0));
    }

    /// One board failing, or never answering, doesn't stop the others getting the message.
    #[tokio::test]
    async fn partial_failure() {
        let got = Arc::new(Mutex::new(vec![]));
        let items = vec!["ok1", "broken", "slow", "ok2"];
        let g = got.clone();
        let results = fan_out(items, Duration::from_millis(200), move |id| {
            let g = g.clone();
            async move {
                match id {
                    "broken" => Err("Board answered HTTP 500".to_string()),
                    "slow" => {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        Ok(())
                    }
                    _ => {
                        g.lock().unwrap().push(id);
                        Ok(())
                    }
                }
            }
        })
        .await;
        let mut got = got.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, ["ok1", "ok2"]);
        assert_eq!(results.iter().map(|r| r.0).collect::<Vec<_>>(), ["ok1", "broken", "slow", "ok2"]);
        assert!(results[0].1.is_ok() && results[3].1.is_ok());
        assert_eq!(results[1].1.as_ref().unwrap_err(), "Board answered HTTP 500");
        assert!(results[2].1.as_ref().unwrap_err().contains("in time"));
        assert_eq!(status_of("The board is rate-limiting messages; wait a little"), "rate-limited");
    }

    /// Over HTTP: a Rattleboard that works, one with the wrong key and one that's offline.
    #[tokio::test]
    async fn partial_failure_over_http() {
        use crate::sequence::Sequence;
        use crate::server::SequenceStatus;
        struct Tv(Mutex<Option<Grid>>, (usize, usize));
        impl server::Board for Tv {
            fn api_key(&self) -> String {
                "k".into()
            }
            fn dims(&self) -> (usize, usize) {
                self.1
            }
            fn adaptive(&self) -> bool {
                false
            }
            fn name(&self) -> String {
                "TV".into()
            }
            fn current(&self) -> (Grid, Option<String>) {
                (vec![], None)
            }
            fn post(&self, g: Grid, _: Transition) -> String {
                *self.0.lock().unwrap() = Some(g);
                "m".into()
            }
            fn play(&self, _: Sequence) -> String {
                String::new()
            }
            fn sequence(&self) -> Option<SequenceStatus> {
                None
            }
            fn stop(&self) {}
        }
        let tv = Arc::new(Tv(Mutex::new(None), (3, 15)));
        let small = Arc::new(Tv(Mutex::new(None), (6, 22)));
        let h1 = server::start(tv.clone(), 0, false).await.unwrap();
        let h2 = server::start(small.clone(), 0, false).await.unwrap();
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let mk = |id: &str, host: String| SavedBoard { host, ..board(id, 6, 22) };
        let boards = vec![
            (mk("tv", h1.addr.to_string()), "k"),
            (mk("wrong", h2.addr.to_string()), "bad"),
            (mk("offline", closed.to_string()), "k"),
        ];
        let c = vestaboard::client();
        let grid = vec![vec![5u8; 15]; 3];
        let results = fan_out(boards, TIMEOUT, move |(b, key)| {
            let (c, grid) = (c.clone(), grid.clone());
            async move { post(&c, &b, key, &grid, &Transition::default()).await }
        })
        .await;
        assert!(results[0].1.is_ok());
        assert_eq!(*tv.0.lock().unwrap(), Some(vec![vec![5u8; 15]; 3]));
        assert_eq!(results[1].1.as_ref().unwrap_err(), "Key was rejected");
        assert!(results[2].1.is_err());
        assert!(small.0.lock().unwrap().is_none());
        drop((h1, h2));
    }
}
