//! The board itself: what it shows, sequences, and the Sources rotation, with every change
//! pushed to the browsers watching it. The desktop app does the same in its lib.rs, around
//! windows; here there are none, only viewers.

use crate::config::Config;
use rattleboard_core::sequence::Sequence;
use rattleboard_core::server::{self, SequenceStatus};
use rattleboard_core::sources::{self, Engine};
use rattleboard_core::vestaboard::{self, Grid, Transition};
use rattleboard_core::now_ms;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// What every viewer is sent, whenever it changes. No keys, no settings beyond looks.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    pub display: Grid,
    pub transition: Transition,
    /// Goes up when the same message is put up again, so the flaps replay
    pub seq: u64,
    pub finish: String,
    /// The source on the board, if the rotation is showing one
    pub showing: Option<String>,
}

/// How something gets onto the board (as on the desktop).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shown {
    /// From the API or a sequence: holds the sources rotation for a while
    Message,
    /// The rotation's next card, or a live card re-drawn
    Ambient,
    /// A source's alert: what was there is put back afterwards
    Alert,
    /// A timed notification: like an alert, it never holds the rotation
    Notice,
    /// What was there before an alert, or before a restart
    Restore,
}

/// A timed notification showing: its id, and what goes back when it's over (layout, how its
/// flaps start, message id, link).
type NoticeUp = (String, Grid, Transition, Option<String>, Option<String>);

#[derive(Default)]
struct Live {
    layout: Option<Grid>,
    id: Option<String>,
    transition: Transition,
    seq: u64,
    api_seq: u64,
    alert_return: Option<(Grid, Transition)>,
    /// The web address of what's showing, if it has one
    link: Option<String>,
    /// A timed notification showing: its id, and what goes back when it's over
    notice: Option<NoticeUp>,
}

struct Player {
    status: SequenceStatus,
    task: tokio::task::JoinHandle<()>,
}

/// The last message, kept in the data folder so a restart shows it again.
#[derive(Serialize, Deserialize)]
struct Saved {
    layout: Grid,
    #[serde(default)]
    transition: Transition,
}

pub struct Board {
    pub cfg: Config,
    key: String,
    pub viewer_token: Option<String>,
    live: Mutex<Live>,
    player: Mutex<Option<Player>>,
    pub engine: Arc<Engine>,
    view: watch::Sender<Arc<View>>,
    saved_path: PathBuf,
}

impl Board {
    pub fn new(cfg: Config, key: String, viewer_token: Option<String>) -> Arc<Board> {
        let engine = Arc::new(Engine::without_commands(sources::Paths {
            config: Some(cfg.data.join("sources.json")),
            plugins: Some(cfg.data.join("plugins.json")),
            seen: Some(cfg.data.join("seen.json")),
        }));
        let saved_path = cfg.data.join("board.json");
        let mut live = Live::default();
        if let Some(s) = std::fs::read_to_string(&saved_path).ok().and_then(|s| serde_json::from_str::<Saved>(&s).ok()) {
            // Only a layout that still fits this board's size, unless it's adaptive.
            if cfg.adaptive || vestaboard::dims_of(&s.layout) == cfg.dims() {
                live.layout = Some(s.layout);
                live.transition = s.transition;
            }
        }
        let (view, _) = watch::channel(Arc::new(View {
            name: cfg.name.clone(),
            rows: cfg.rows,
            cols: cfg.cols,
            display: vec![],
            transition: Transition::default(),
            seq: 0,
            finish: cfg.finish.clone(),
            showing: None,
        }));
        let b = Arc::new(Board { cfg, key, viewer_token, live: Mutex::new(live), player: Mutex::new(None), engine, view, saved_path });
        b.publish();
        let weak = Arc::downgrade(&b);
        b.engine.on_change(Arc::new(move || {
            if let Some(b) = weak.upgrade() {
                b.publish();
            }
        }));
        b
    }

    pub fn api_key(&self) -> &str {
        &self.key
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<View>> {
        self.view.subscribe()
    }

    pub fn view(&self) -> Arc<View> {
        self.view.borrow().clone()
    }

    fn blank(&self) -> Grid {
        vec![vec![0; self.cfg.cols]; self.cfg.rows]
    }

    /// Sends viewers the board as it is now, if anything they see changed.
    fn publish(&self) {
        let (display, transition, seq) = {
            let l = self.live.lock().unwrap();
            (l.layout.clone().unwrap_or_else(|| self.blank()), l.transition.clone(), l.seq)
        };
        // An adaptive board is the size of whatever it shows.
        let (rows, cols) = if self.cfg.adaptive { vestaboard::dims_of(&display) } else { self.cfg.dims() };
        let showing = self.engine.status().showing;
        let v = View { name: self.cfg.name.clone(), rows, cols, display, transition, seq, finish: self.cfg.finish.clone(), showing };
        self.view.send_if_modified(|cur| {
            if **cur == v {
                return false;
            }
            *cur = Arc::new(v);
            true
        });
    }

    fn next_id(&self, prefix: &str) -> String {
        let mut l = self.live.lock().unwrap();
        l.api_seq += 1;
        format!("{prefix}-{}-{}", now_ms(), l.api_seq)
    }

    /// Puts a layout on the board for every viewer.
    pub fn put(&self, layout: Grid, id: Option<String>, tr: Transition, how: Shown, link: Option<String>) {
        if how == Shown::Message {
            self.engine.note_message(now_ms());
        }
        let saved = {
            let mut l = self.live.lock().unwrap();
            if l.layout.as_ref() == Some(&layout) {
                // The same picture again (an app re-sent it): replay the flip.
                l.seq += 1;
            }
            match how {
                // Nothing to go back to: the message replaced the alert, or the notification.
                Shown::Message => {
                    l.alert_return = None;
                    l.notice = None;
                }
                Shown::Alert if l.alert_return.is_none() => l.alert_return = l.layout.clone().map(|g| (g, l.transition.clone())),
                _ => {}
            }
            l.layout = Some(layout.clone());
            l.id = id;
            l.link = link;
            l.transition = tr.clone();
            Saved { layout, transition: tr }
        };
        if let Ok(s) = serde_json::to_string(&saved) {
            let _ = std::fs::write(&self.saved_path, s);
        }
        self.publish();
    }

    // ------------------------------------------------------------ notifications

    /// A notification (see `server::Notice`), as on the desktop: one that stays is a
    /// message; a timed one interrupts like an alert, then what it covered goes back quietly,
    /// never as a new message.
    fn notify(self: &Arc<Self>, n: server::Notice) -> String {
        let id = self.next_id("note");
        self.stop_player();
        let Some(secs) = n.seconds else {
            self.put(n.layout, Some(id.clone()), n.tr, Shown::Message, n.link);
            return id;
        };
        let flip = {
            let mut l = self.live.lock().unwrap();
            let from = l.layout.clone().unwrap_or_else(|| self.blank());
            let back = match l.notice.take() {
                Some((_, g, tr, mid, link)) => (g, tr, mid, link),
                None => (from.clone(), l.transition.clone(), l.id.clone(), l.link.clone()),
            };
            l.notice = Some((id.clone(), back.0, back.1, back.2, back.3));
            vestaboard::flip_ms(&from, &n.layout, &n.tr, 1.0)
        };
        let ms = flip + secs * 1000;
        self.engine.begin_notice(now_ms(), ms as i64);
        self.put(n.layout, Some(id.clone()), n.tr, Shown::Notice, n.link);
        let (b, nid) = (self.clone(), id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            b.end_notice(&nid);
        });
        id
    }

    fn end_notice(&self, id: &str) {
        let back = {
            let mut l = self.live.lock().unwrap();
            match &l.notice {
                Some((nid, ..)) if nid == id => l.notice.take(),
                _ => None,
            }
        };
        let Some((_, layout, tr, mid, link)) = back else { return };
        // The very picture that was up: nothing to put back (that would replay its flaps).
        if self.live.lock().unwrap().layout.as_ref() != Some(&layout) {
            let mid = mid.or_else(|| Some(self.next_id("msg")));
            self.put(layout, mid, tr, Shown::Restore, link);
        }
        self.engine.end_notice(now_ms());
    }

    // ------------------------------------------------------------ sequences

    fn stop_player(&self) {
        if let Some(p) = self.player.lock().unwrap().take() {
            p.task.abort();
        }
    }

    fn start_player(self: &Arc<Self>, seq: Sequence) -> String {
        let id = self.next_id("seq");
        let status = SequenceStatus { id: id.clone(), name: seq.name.clone(), frame: 1, frames: seq.frames.len(), looping: seq.looping };
        // Held while spawning, so the task can't report a frame before it's registered.
        let mut player = self.player.lock().unwrap();
        let task = tokio::spawn(play(self.clone(), id.clone(), seq));
        if let Some(old) = player.replace(Player { status, task }) {
            old.task.abort();
        }
        id
    }

    /// Marks frame `n` as showing; false once this sequence has been replaced or stopped.
    fn on_frame(&self, id: &str, n: usize) -> bool {
        match self.player.lock().unwrap().as_mut() {
            Some(p) if p.status.id == id => {
                p.status.frame = n;
                true
            }
            _ => false,
        }
    }

    fn end_player(&self, id: &str) {
        let mut player = self.player.lock().unwrap();
        if player.as_ref().is_some_and(|p| p.status.id == id) {
            *player = None;
        }
    }

    // ------------------------------------------------------------ sources

    pub fn sources_ctx(&self) -> sources::Ctx {
        sources::Ctx { now: now_ms(), local: chrono::Local::now().naive_local(), paused: None, dims: self.cfg.dims() }
    }

    fn run_actions(&self, actions: Vec<sources::Action>) {
        for a in actions {
            match a {
                // There are no other boards to drive here: every card is for this one.
                sources::Action::Show { card, page, how } => {
                    let (rows, cols) = self.cfg.dims();
                    let mut pages = card.face.render(rows, cols);
                    let layout = pages.swap_remove(page.min(pages.len() - 1));
                    let shown = match how {
                        sources::How::Alert => Shown::Alert,
                        sources::How::Message => Shown::Restore,
                        _ => Shown::Ambient,
                    };
                    self.put(layout, Some(self.next_id("src")), Transition::default(), shown, card.link.clone());
                }
                sources::Action::Return => {
                    let back = self.live.lock().unwrap().alert_return.take();
                    if let Some((layout, tr)) = back {
                        self.put(layout, Some(self.next_id("src")), tr, Shown::Restore, None);
                    }
                }
            }
        }
    }

    /// The rotation: fetches run in tasks of their own, so a slow feed never holds it up; it
    /// moves on once a second, or at once when something changes.
    pub async fn sources_loop(self: Arc<Self>) {
        let engine = self.engine.clone();
        loop {
            let ctx = self.sources_ctx();
            for (id, job) in engine.due_jobs(ctx.now) {
                let (e, local, now) = (engine.clone(), ctx.local, ctx.now);
                tokio::spawn(async move {
                    let r = e.run_job(job, local, now).await;
                    e.store(&id, r, now_ms());
                });
            }
            let actions = engine.tick(&ctx);
            self.run_actions(actions);
            self.publish();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = engine.wake.notified() => {}
            }
        }
    }
}

/// Each frame is held for `hold` seconds after its flaps finish, worked out from the
/// replica's own timing, as on the desktop.
async fn play(b: Arc<Board>, id: String, seq: Sequence) {
    let mut i = 0;
    loop {
        let f = &seq.frames[i];
        if !b.on_frame(&id, i + 1) {
            return;
        }
        let from = {
            let l = b.live.lock().unwrap();
            match &l.layout {
                // The same picture again flips from blank.
                Some(g) if *g != f.layout => g.clone(),
                _ => b.blank(),
            }
        };
        let flip = vestaboard::flip_ms(&from, &f.layout, &f.tr, 1.0);
        b.put(f.layout.clone(), Some(format!("{id}-{}", i + 1)), f.tr.clone(), Shown::Message, f.link.clone());
        tokio::time::sleep(Duration::from_millis((flip as f64 + f.hold * 1000.0) as u64)).await;
        i += 1;
        if i == seq.frames.len() {
            if !seq.looping {
                break;
            }
            i = 0;
        }
    }
    b.end_player(&id);
}

/// The API server's view of this board.
pub struct Api(pub Arc<Board>);

impl server::Board for Api {
    fn api_key(&self) -> String {
        self.0.key.clone()
    }
    fn dims(&self) -> (usize, usize) {
        self.0.cfg.dims()
    }
    fn adaptive(&self) -> bool {
        self.0.cfg.adaptive
    }
    fn name(&self) -> String {
        self.0.cfg.name.clone()
    }
    fn current(&self) -> (Grid, Option<String>) {
        let l = self.0.live.lock().unwrap();
        (l.layout.clone().unwrap_or_else(|| self.0.blank()), l.id.clone())
    }
    fn post(&self, layout: Grid, tr: Transition, link: Option<String>) -> String {
        let id = self.0.next_id("api");
        self.0.stop_player();
        self.0.put(layout, Some(id.clone()), tr, Shown::Message, link);
        id
    }
    fn link(&self) -> Option<String> {
        self.0.live.lock().unwrap().link.clone()
    }
    fn notify(&self, n: server::Notice) -> String {
        self.0.notify(n)
    }
    fn play(&self, seq: Sequence) -> String {
        self.0.start_player(seq)
    }
    fn sequence(&self) -> Option<SequenceStatus> {
        self.0.player.lock().unwrap().as_ref().map(|p| p.status.clone())
    }
    fn stop(&self) {
        self.0.stop_player();
    }
    fn sources(&self) -> Option<Arc<Engine>> {
        Some(self.0.engine.clone())
    }
    fn sources_ctx(&self) -> Option<sources::Ctx> {
        Some(self.0.sources_ctx())
    }
    fn platform(&self) -> &'static str {
        "web"
    }
    fn index(&self) -> Option<axum::response::Response> {
        Some(crate::page::index())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use rattleboard_core::server::Board as _;

    fn board(name: &str, vars: &[(&str, &str)]) -> Arc<Board> {
        let dir = std::env::temp_dir().join(format!("rattleboard-web-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        vars.push(("RATTLEBOARD_DATA".into(), dir.to_string_lossy().into()));
        let cfg = Config::from(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap();
        Board::new(cfg, "k".repeat(24), None)
    }

    #[tokio::test]
    async fn posts_reach_viewers_and_survive_a_restart() {
        let b = board("post", &[("RATTLEBOARD_SIZE", "compact")]);
        let mut rx = b.subscribe();
        assert_eq!(rx.borrow_and_update().display, vec![vec![0u8; 15]; 3]);
        let api = Api(b.clone());
        let g = vestaboard::text_layout("HELLO", 3, 15);
        let id = api.post(g.clone(), Transition::default(), None);
        assert!(id.starts_with("api-"));
        assert!(rx.has_changed().unwrap());
        let v = rx.borrow_and_update().clone();
        assert_eq!((v.display.clone(), v.seq, v.rows, v.cols), (g.clone(), 0, 3, 15));
        // The same message again replays the flip.
        api.post(g.clone(), Transition::default(), None);
        assert_eq!(rx.borrow_and_update().seq, 1);
        assert_eq!(api.current().0, g);
        assert_eq!(api.platform(), "web");
        // A new board on the same data folder starts with it.
        let again = Board::new(b.cfg.clone(), "k".repeat(24), None);
        assert_eq!(again.view().display, g);
    }

    #[tokio::test]
    async fn alerts_put_back_what_was_there() {
        let b = board("alert", &[]);
        let before = vestaboard::text_layout("BEFORE", 6, 22);
        b.put(before.clone(), None, Transition::default(), Shown::Message, None);
        b.put(vestaboard::text_layout("ALERT", 6, 22), None, Transition::default(), Shown::Alert, None);
        b.run_actions(vec![sources::Action::Return]);
        assert_eq!(b.view().display, before);
    }

    /// A timed notification (an agent's show_status) goes back to what was there, once; a
    /// second one while it's up still goes back to the first thing; a message replaces it.
    #[tokio::test(start_paused = true)]
    async fn notifications_go_back_once() {
        use rattleboard_core::server::Notice;
        let b = board("notice", &[]);
        let api = Api(b.clone());
        let g = |t: &str| vestaboard::text_layout(t, 6, 22);
        let note = |t: &str, seconds: Option<u64>| Notice { layout: g(t), tr: Transition::default(), seconds, source: "Claude Code via MCP".into(), link: None, alert: false };
        api.post(g("BEFORE"), Transition::default(), None);
        assert!(api.notify(note("RUNNING TESTS", Some(5))).starts_with("note-"));
        assert_eq!(b.view().display, g("RUNNING TESTS"));
        tokio::time::sleep(Duration::from_secs(3)).await;
        api.notify(note("DEPLOYING", Some(5)));
        assert_eq!(b.view().display, g("DEPLOYING"));
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert_eq!(b.view().display, g("DEPLOYING"), "the second one has its own time");
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(b.view().display, g("BEFORE"));
        // and nothing more, however long it's left
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(b.view().display, g("BEFORE"));

        // A message while one is up replaces it: nothing comes back over the message.
        api.notify(note("STATUS", Some(5)));
        api.post(g("NEW MESSAGE"), Transition::default(), None);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(b.view().display, g("NEW MESSAGE"));
        // The same picture as was up: no second flip when it "goes back".
        let seq = b.view().seq;
        api.notify(note("NEW MESSAGE", Some(5)));
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!((b.view().display.clone(), b.view().seq), (g("NEW MESSAGE"), seq + 1));
        // One that stays (send_message) just stays.
        api.notify(note("BUILD PASSED", None));
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(b.view().display, g("BUILD PASSED"));
    }

    #[tokio::test(start_paused = true)]
    async fn sequences_play_and_stop() {
        let b = board("seq", &[]);
        let api = Api(b.clone());
        let v = serde_json::json!({"name": "Two", "loop": false, "frames": [{"text": "ONE", "hold": 1}, {"text": "TWO", "hold": 1}]});
        let seq = rattleboard_core::sequence::parse(&v, (6, 22), false).unwrap();
        api.play(seq);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(api.sequence().map(|s| (s.frame, s.frames)), Some((1, 2)));
        assert_eq!(b.view().display, vestaboard::text_layout("ONE", 6, 22));
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(b.view().display, vestaboard::text_layout("TWO", 6, 22));
        assert!(api.sequence().is_none(), "a sequence without loop ends");
        let looping = serde_json::json!({"loop": true, "frames": [{"text": "A", "hold": 1}]});
        api.play(rattleboard_core::sequence::parse(&looping, (6, 22), false).unwrap());
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(api.sequence().is_some());
        // A posted message stops it.
        api.post(vestaboard::text_layout("STOP", 6, 22), Transition::default(), None);
        assert!(api.sequence().is_none());
    }
}
