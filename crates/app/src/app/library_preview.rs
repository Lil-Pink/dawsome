//! The Library's preview: a single click on an instrument row (a starter kit sound, an
//! instrument set or a plugin instrument) makes it the row the typing keyboard and the MIDI
//! devices no link routes play, before it is added to the rack. A channel of the plans alone
//! plays it ([`Session::set_preview`](crate::session::Session::set_preview), under
//! [`PREVIEW_CHANNEL`]): never in the document, its saves, its exports, the rack or the
//! history, at the default sound settings, into the master. Its row shows a mark while it is
//! the preview. It ends when a rack channel is selected, an instrument is added, another part
//! of the Library is pressed, or another document opens; its notes end with it. A plugin's
//! preview instance is the only one: the next preview replaces it.

use dawsome_engine::project::{Channel, ChannelId};

use super::plugins::plugin_processor;
use super::App;
use crate::session::{SessionError, PREVIEW_CHANNEL};
use crate::ui::browser::Pick;
use crate::ui::browser_model::Playing;
use crate::ui::library::InstrumentSet;

/// What the hint bar says while the Library previews a row and nothing is under the cursor,
/// naming the key that switches the computer keyboard as bound.
pub fn preview_hint(keys: &crate::keys::Keys) -> String {
    format!(
        "Playing from the Library \u{b7} {} or your keyboard \u{b7} double-click to add",
        keys.first(crate::keys::Action::Keyboard)
    )
}

impl App {
    /// Makes `pick` (a kit sound, a set or a plugin instrument) the Library's preview: its own
    /// channel plays the typing keyboard's notes and the devices' no link routes from now on.
    /// Anything else, or a kit sound with no folder to be written to, previews nothing.
    pub(super) fn preview_in_library(&mut self, pick: &Pick) -> Result<(), SessionError> {
        let Some((playing, channel)) = self.preview_of(pick) else {
            return Ok(());
        };
        self.session.set_preview(Some(channel))?;
        self.browse.model.playing = Some(playing);
        self.browse.cache.clear();
        self.route_ports();
        // A previewed plugin that could not be created says so now, as a rack plugin does.
        self.show_plugin_notices();
        Ok(())
    }

    /// Previews the instrument set whose main file is `sfz` as a press on its Library row
    /// would: what `--library-preview` asks for at start.
    pub fn preview_set_at_start(&mut self, sfz: &std::path::Path) {
        let name = sfz
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let set = InstrumentSet {
            name,
            maker: String::new(),
            sfz: sfz.to_path_buf(),
            bytes: None,
        };
        let result = self.preview_in_library(&Pick::Set(set));
        self.report(result);
    }

    /// Ends the Library's preview, its notes with it: the typing keyboard's held keys end, and
    /// the plans drop its channel, whose voices end with it. Nothing when there is none.
    pub(super) fn end_library_preview(&mut self) -> Result<(), SessionError> {
        if self.browse.model.playing.is_none() {
            return Ok(());
        }
        self.release_typed_keys();
        self.session.set_preview(None)?;
        self.browse.model.playing = None;
        self.browse.cache.clear();
        self.route_ports();
        self.show_plugin_notices();
        Ok(())
    }

    /// Forgets the Library's preview with the document it played beside: the next document's
    /// session has none.
    pub(super) fn forget_library_preview(&mut self) {
        if self.browse.model.playing.take().is_some() {
            self.release_typed_keys();
            self.browse.cache.clear();
        }
    }

    /// The channel the live notes no link routes play now: the Library's preview, whatever the
    /// rack selects; else the instrument window's while it is the selected one, which it is
    /// but after a press let go of off a channel's name (that selects and leaves the window
    /// where it is, [`App::press_channel`]); none plays the selected one.
    pub(super) fn live_target(&self) -> Option<ChannelId> {
        match self.browse.model.playing {
            Some(_) => Some(PREVIEW_CHANNEL),
            None => self
                .instrument_on()
                .filter(|&shown| Some(shown) == self.selected_channel),
        }
    }

    /// What previewing `pick` marks in the Library, and the channel that plays it: as the rack
    /// would add it, at the default sound settings (a set with those it is added with: the 808
    /// basses cutting themselves).
    fn preview_of(&mut self, pick: &Pick) -> Option<(Playing, Channel)> {
        Some(match pick {
            Pick::Sound(index) => {
                let (file, name) = self.kit_file(*index)?;
                let channel = Channel::sampler(name, self.sample_path(&file)).as_added();
                (Playing::KitSound(*index), channel)
            }
            Pick::Set(set) => {
                let mut channel =
                    Channel::instrument(set.name.clone(), self.sample_path(&set.sfz)).as_added();
                channel.sound = crate::sets::added_sound(&set.sfz);
                (Playing::Set(set.sfz.clone()), channel)
            }
            Pick::Plugin(plugin) => {
                let channel = Channel::plugin(plugin.plugin.name.clone(), plugin_processor(plugin))
                    .as_added();
                (Playing::Plugin(plugin.processor_id()), channel)
            }
            // An input plays through its own channel once added: there is nothing to preview.
            Pick::ExternalInput
            | Pick::Rescan
            | Pick::Device(_)
            | Pick::Tick(_)
            | Pick::Group(_) => return None,
        })
    }
}
