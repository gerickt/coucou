//! mochi/frames/manifest.json: the desktop frame sequences that
//! tools/render-mochi renders from the original engine (the same data as
//! manifest.luau, which the Noctalia widgets require).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

/// One sequence: `frames` are (file relative to the plugin directory, hold),
/// a hold counting frames at `fps`.
#[derive(Debug, Clone, PartialEq)]
pub struct Seq {
    pub fps: f64,
    pub looped: bool,
    pub frames: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    /// Side of every frame, in pixels.
    pub px: u32,
    /// "<outfit>.<name>" -> sequence.
    pub seqs: HashMap<String, Arc<Seq>>,
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("manifest.json: {e}"))?;
    let desktop = root.get("desktop").ok_or("manifest.json: no \"desktop\"")?;
    let px = desktop.get("px").and_then(Value::as_u64).ok_or("manifest.json: no px")? as u32;
    let list = desktop
        .get("sequences")
        .and_then(Value::as_object)
        .ok_or("manifest.json: no sequences")?;
    let mut seqs = HashMap::with_capacity(list.len());
    for (name, q) in list {
        let fps = q.get("fps").and_then(Value::as_f64).filter(|f| *f > 0.0);
        let looped = q.get("loop").and_then(Value::as_bool);
        let frames = q.get("frames").and_then(Value::as_array);
        let (Some(fps), Some(looped), Some(frames)) = (fps, looped, frames) else {
            return Err(format!("manifest.json: bad sequence {name}"));
        };
        let mut out = Vec::with_capacity(frames.len());
        for f in frames {
            let file = f.get(0).and_then(Value::as_str);
            let hold = f.get(1).and_then(Value::as_u64).filter(|h| *h > 0);
            match (file, hold) {
                // Files stay inside the plugin's frames/ directory.
                (Some(file), Some(hold)) if file.starts_with("frames/") && !file.contains("..") => {
                    out.push((file.to_string(), hold as u32))
                }
                _ => return Err(format!("manifest.json: bad frame in {name}")),
            }
        }
        if out.is_empty() {
            return Err(format!("manifest.json: empty sequence {name}"));
        }
        seqs.insert(name.clone(), Arc::new(Seq { fps, looped, frames: out }));
    }
    Ok(Manifest { px, seqs })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"desktop":{"px":180,"sequences":{
        "none.idle":{"fps":12.0,"loop":true,"frames":[["frames/d/a.png",26],["frames/d/b.png",1]]},
        "none.slap":{"fps":15.0,"loop":false,"frames":[["frames/d/c.png",3]]}}}}"#;

    #[test]
    fn parses_sequences() {
        let m = parse(SAMPLE).unwrap();
        assert_eq!(m.px, 180);
        let idle = &m.seqs["none.idle"];
        assert!(idle.looped);
        assert_eq!(idle.fps, 12.0);
        assert_eq!(idle.frames, vec![("frames/d/a.png".into(), 26), ("frames/d/b.png".into(), 1)]);
        assert!(!m.seqs["none.slap"].looped);
    }

    #[test]
    fn refuses_paths_outside_frames() {
        let bad = SAMPLE.replace("frames/d/c.png", "../../etc/passwd");
        assert!(parse(&bad).is_err());
        let bad = SAMPLE.replace("frames/d/c.png", "frames/../../x.png");
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn refuses_zero_hold_and_missing_fields() {
        assert!(parse(&SAMPLE.replace("26]", "0]")).is_err());
        assert!(parse(r#"{"desktop":{"px":180}}"#).is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn shipped_manifest_has_every_state_and_gaze() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../mochi/frames/manifest.json");
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let m = parse(&text).unwrap();
        for s in ["idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping", "dizzy"] {
            assert!(m.seqs.contains_key(&format!("none.{s}")), "none.{s}");
        }
        for d in crate::gaze::Dir::ALL {
            assert!(m.seqs.contains_key(&format!("none.idle.look.{}", d.name())));
            assert!(m.seqs.contains_key(&format!("crown.working.look.{}", d.name())));
        }
    }
}
