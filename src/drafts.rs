// SPDX-License-Identifier: MIT

//! What has been written and not published: one text for each thing that
//! can be written to.
//!
//! Leaving the composer keeps the text, so that coming back to the same
//! note finds it. What is kept here lasts as long as the session. What is
//! there after a restart is what the relay Avestruz runs itself keeps (see
//! [`crate::nostr::kept_drafts`]), which every draft is sent to when the
//! composer is left; and, when that relay can't be given the draft of a
//! new note, the file that draft was kept in before there was such a
//! relay.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::nostr::compose::Target;

/// The unpublished text for each target that has any.
#[derive(Debug, Default)]
pub struct Drafts {
    texts: HashMap<Target, String>,
}

impl Drafts {
    /// What was written to `target` and left there. Empty if nothing.
    pub fn get(&self, target: Target) -> &str {
        self.texts.get(&target).map_or("", String::as_str)
    }

    /// Keep `text` as what is written to `target`. Text that is blank is no
    /// draft: nothing is kept.
    pub fn set(&mut self, target: Target, text: String) {
        if text.trim().is_empty() {
            self.texts.remove(&target);
        } else {
            self.texts.insert(target, text);
        }
    }
}

/// Where the draft of a new note is kept: the user's data directory, beside
/// the event store.
pub fn default_draft_path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("cosmic-applet-avestruz").join("draft.txt")
}

/// The draft kept at `path`. No file is no draft.
pub async fn load(path: PathBuf) -> String {
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            tracing::warn!("the draft at {} could not be read: {e}", path.display());
            String::new()
        }
    }
}

/// Write `text` to `path` in a way that leaves either the old draft or the
/// new one, never half of one.
async fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        tokio::fs::create_dir_all(directory).await?;
    }
    let partial = path.with_extension("partial");
    tokio::fs::write(&partial, text).await?;
    tokio::fs::rename(&partial, path).await
}

/// Keep `text` at `path` as the draft of a new note. A blank text is no
/// draft, and takes the file away.
pub async fn save(path: PathBuf, text: String) {
    let result = if text.trim().is_empty() {
        match tokio::fs::remove_file(&path).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        write(&path, &text).await
    };
    if let Err(e) = result {
        tracing::warn!("the draft could not be kept at {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::{Drafts, load, save};
    use crate::nostr::compose::Target;
    use nostr_sdk::prelude::EventId;

    #[test]
    fn each_target_has_its_own_draft_and_a_blank_one_is_none() {
        let (one, other) = (Target::Reply(EventId::from_byte_array([1; 32])), Target::Reply(EventId::from_byte_array([2; 32])));
        let mut drafts = Drafts::default();
        assert_eq!(drafts.get(Target::Note), "");

        drafts.set(Target::Note, "a note".into());
        drafts.set(one, "a reply".into());
        assert_eq!(drafts.get(Target::Note), "a note");
        assert_eq!(drafts.get(one), "a reply");
        assert_eq!(drafts.get(other), "");

        drafts.set(one, "  \n ".into());
        assert_eq!(drafts.get(one), "");
        assert_eq!(drafts.get(Target::Note), "a note");
    }

    #[test]
    fn the_draft_of_a_note_is_there_after_a_restart_until_it_is_blank() {
        let directory = std::env::temp_dir().join(format!("avestruz-test-{}-draft", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("nested").join("draft.txt");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

        runtime.block_on(async {
            assert_eq!(load(path.clone()).await, "", "no file, no draft");

            save(path.clone(), "half a thought\nand a line".into()).await;
            assert_eq!(load(path.clone()).await, "half a thought\nand a line");

            save(path.clone(), "   ".into()).await;
            assert_eq!(load(path.clone()).await, "");
            assert!(!path.exists(), "a blank draft leaves no file");
            // And taking away what is not there is not an error.
            save(path.clone(), String::new()).await;
        });
        let _ = std::fs::remove_dir_all(&directory);
    }
}
