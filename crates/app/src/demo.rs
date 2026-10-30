//! The demo project: a short song made only of sounds the app's own generator synthesized
//! (examples/gen_demo.py), carried in the binary. It is written into the user's demo folder the
//! first time it is needed and opened from there, so it opens wherever the app runs, and changes
//! to it never reach the files it was made from.

use std::fs;
use std::path::{Path, PathBuf};

use crate::project_file::{self, SaveError};

/// The demo's project file, by its name in the demo folder.
const PROJECT: &str = "demo.json";

/// Every file of the demo, by its path inside the demo folder.
const FILES: [(&str, &[u8]); 5] = [
    (PROJECT, include_bytes!("../../../examples/demo.json")),
    (
        "sounds/kick.wav",
        include_bytes!("../../../examples/sounds/kick.wav"),
    ),
    (
        "sounds/snare.wav",
        include_bytes!("../../../examples/sounds/snare.wav"),
    ),
    (
        "sounds/hat.wav",
        include_bytes!("../../../examples/sounds/hat.wav"),
    ),
    (
        "sounds/click.wav",
        include_bytes!("../../../examples/sounds/click.wav"),
    ),
];

/// The demo's project file in `dir`, after writing each demo file `dir` lacks. A file already
/// there is left as it is: the copy is the user's to change.
pub fn install(dir: &Path) -> Result<PathBuf, SaveError> {
    for (name, bytes) in FILES {
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| SaveError::Write {
                path: path.clone(),
                source,
            })?;
        }
        project_file::write_atomically(&path, bytes)?;
    }
    Ok(dir.join(PROJECT))
}
