//! Sequences: frames a board plays by itself, each held for a while once its flaps finish.
//! Shared by the API server (POST /rattleboard/sequence) and Compose.
//!
//!   {"name": "Morning", "loop": true,
//!    "frames": [{"characters": [[...]], "hold": 8, "strategy": "column"},
//!               {"text": "Coffee is ready", "hold": 5}]}

use crate::vestaboard::{Grid, Transition};
use serde_json::{json, Map, Value};

pub const MAX_FRAMES: usize = 100;
pub const MAX_HOLD: f64 = 3600.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub layout: Grid,
    /// Seconds, counted from when the frame's flaps finish
    pub hold: f64,
    pub tr: Transition,
    /// The web address a click opens while this frame shows
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub name: String,
    pub looping: bool,
    pub frames: Vec<Frame>,
}

/// Validates a posted sequence for a rows×cols board, each frame laid out as a normal post
/// would be (see `server::message`).
pub fn parse(v: &Value, (rows, cols): (usize, usize), adaptive: bool) -> Result<Sequence, String> {
    let o = v.as_object().ok_or("Send {\"frames\": [...]}")?;
    let name = match o.get("name") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(_) => return Err("name must be a string".into()),
    };
    let looping = match o.get("loop") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("loop must be true or false".into()),
    };
    let list = o.get("frames").and_then(|f| f.as_array()).ok_or("frames must be an array")?;
    if list.is_empty() || list.len() > MAX_FRAMES {
        return Err(format!("A sequence needs 1 to {MAX_FRAMES} frames"));
    }
    let frames = list
        .iter()
        .enumerate()
        .map(|(i, f)| frame(f, (rows, cols), adaptive).map_err(|e| format!("Frame {}: {e}", i + 1)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Sequence { name, looping, frames })
}

fn frame(v: &Value, dims: (usize, usize), adaptive: bool) -> Result<Frame, String> {
    let o = v.as_object().ok_or("each frame must be an object")?;
    if o.get("characters").is_none() && !o.get("text").is_some_and(|t| t.is_string()) {
        return Err("needs \"characters\" or \"text\"".into());
    }
    // A frame is laid out like a normal post (size hint and all), minus the hold.
    let (layout, tr) = crate::server::message(v, dims, adaptive)?;
    let hold = o.get("hold").and_then(|h| h.as_f64()).ok_or("needs \"hold\" in seconds")?;
    if !(1.0..=MAX_HOLD).contains(&hold) {
        return Err(format!("hold must be from 1 to {MAX_HOLD} seconds"));
    }
    Ok(Frame { layout, hold, tr, link: crate::server::link_of(v)? })
}

impl Sequence {
    /// The protocol form, for handing the sequence to another Rattleboard.
    pub fn to_json(&self) -> Value {
        let frames: Vec<Value> = self
            .frames
            .iter()
            .map(|f| {
                let mut o = Map::new();
                o.insert("characters".into(), json!(f.layout));
                o.insert("hold".into(), json!(f.hold));
                if let Some(l) = &f.link {
                    o.insert("link".into(), json!(l));
                }
                f.tr.add_to(&mut o);
                Value::Object(o)
            })
            .collect();
        json!({ "name": self.name, "loop": self.looping, "frames": frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vestaboard;

    #[test]
    fn parsing() {
        let grid = vec![vec![1u8; 15]; 3];
        let v = json!({"name": "Morning", "loop": true, "frames": [
            {"characters": grid, "hold": 8, "strategy": "column"},
            {"text": "coffee", "hold": 2.5, "link": "https://example.com/coffee"}
        ]});
        let s = parse(&v, (3, 15), false).unwrap();
        assert_eq!((s.name.as_str(), s.looping, s.frames.len()), ("Morning", true, 2));
        assert_eq!(s.frames[0].tr.strategy, "column");
        assert_eq!(s.frames[1].layout, vestaboard::text_layout("coffee", 3, 15));
        assert_eq!(parse(&s.to_json(), (3, 15), false).unwrap(), s);
        assert_eq!((s.frames[0].link.as_deref(), s.frames[1].link.as_deref()), (None, Some("https://example.com/coffee")));
        assert!(parse(&json!({"frames": [{"text": "a", "hold": 3, "link": "javascript:x"}]}), (3, 15), false).unwrap_err().contains("link"));

        let bad = |v: Value| parse(&v, (6, 22), false).unwrap_err();
        assert!(bad(json!([])).contains("frames"));
        assert!(bad(json!({"frames": []})).contains("1 to 100"));
        assert!(bad(json!({"frames": vec![json!({"text": "a", "hold": 1}); 101]})).contains("1 to 100"));
        assert!(bad(json!({"frames": [{"text": "a"}]})).contains("hold"));
        assert!(bad(json!({"frames": [{"text": "a", "hold": 0.5}]})).contains("hold"));
        assert!(bad(json!({"frames": [{"text": "a", "hold": 3601}]})).contains("hold"));
        assert!(bad(json!({"frames": [{"hold": 3}]})).contains("characters"));
        assert!(bad(json!({"frames": [{"characters": [[1, 2], [1]], "hold": 3}]})).contains("same length"));
        assert!(bad(json!({"frames": [{"characters": vec![vec![1u8; 5]; 13], "hold": 3}]})).contains("12 rows"));
        // a frame of another size is centred
        let s = parse(&json!({"frames": [{"characters": [[1, 2]], "hold": 3}]}), (3, 15), false).unwrap();
        assert_eq!(s.frames[0].layout[1][6..8], [1, 2]);
        assert!(bad(json!({"frames": [{"text": "a", "hold": 3, "strategy": "wipe"}]})).starts_with("Frame 1"));
        assert!(bad(json!({"loop": "yes", "frames": [{"text": "a", "hold": 3}]})).contains("loop"));
        assert!(bad(json!({"name": 4, "frames": [{"text": "a", "hold": 3}]})).contains("name"));
    }
}
