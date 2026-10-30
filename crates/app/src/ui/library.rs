//! The Library panel's contents, without iced: three sections, top to bottom, each a header row
//! with a count over its own rows, open until its header is pressed.
//!
//! - Instruments: the starter kit, the instrument sets shipped in the data folder (by kind:
//!   drums, bass, keys, pads, strings, mallets, synths, then the rest, each kind under a label of
//!   its own while more than one kind is listed and nothing is searched), the user's own sets
//!   (the .sfz files in the Samples section's folders), then the catalogue's plugin instruments
//!   of both formats grouped by maker (a VST3 one marked "VST3"; one installed as both listed
//!   once, as CLAP), then Rescan; with no plugin instruments, a hint saying where they come
//!   from. A group of sets shows only when it has one. Last, under "Audio input", "External
//!   input": a channel that plays an audio input live (not counted among the section's
//!   sounds).
//! - Samples: the folder tree, or the search's matching files, as [`Browser`] keeps them.
//! - MIDI devices: the input ports listed, each with its tick, its light (lit while it sends) and
//!   where its notes go (the channel linked to it, else the selected one), then the ports a
//!   channel links to that are not plugged in, then the system's loopback ports ("Midi Through",
//!   no device: grayed, still tickable); with none, a hint.
//!
//! Inside Instruments each group folds on its label too ("Starter kit", "Instrument sets" and
//! each kind under it, "My instruments", "Plugins" and each maker under it, "Audio input"): a
//! folded group shows its label alone ([`Library::folded`], by [`fold_key`]). A search shows
//! every match, folded or not.
//!
//! The search box filters every section at once: kit sounds and ports by name, sets and plugins
//! by name or maker as the picker does, and the tree by its own search. Section rows sit in one list the
//! panel scrolls through.
//!
//! [`Browser`]: crate::ui::browser_model::Browser

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use dawsome_engine::project::same_port;
use dawsome_host_clap::catalogue::Offered;

use crate::settings::MidiInputs;
use crate::ui::picker;

/// A section of the Library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Instruments,
    Samples,
    Devices,
}

impl Section {
    /// The name on its header.
    pub fn title(self) -> &'static str {
        match self {
            Section::Instruments => "Instruments",
            Section::Samples => "Samples",
            Section::Devices => "MIDI devices",
        }
    }
}

/// The plugin instruments' empty state, a line per row. Why lines of its own: the panel is too
/// narrow for it in one, and a hint cut with an ellipsis would lose where to put the plugins.
#[cfg(not(windows))]
pub const PLUGIN_HINT: &[&str] = &[
    "No plugin instruments found.",
    "CLAP plugins in /usr/lib/clap or ~/.clap,",
    "VST3 ones in /usr/lib/vst3 or ~/.vst3",
    "show here after Rescan.",
];
/// The plugin instruments' empty state, a line per row, naming the folders the scan reads on
/// Windows.
#[cfg(windows)]
pub const PLUGIN_HINT: &[&str] = &[
    "No plugin instruments found.",
    "CLAP plugins in Common Files\\CLAP or",
    "AppData\\Local\\Programs\\Common\\CLAP,",
    "VST3 ones in Common Files\\VST3",
    "show here after Rescan.",
];
/// The MIDI devices' empty state, a line per row.
pub const DEVICE_HINT: &[&str] = &["Plug in a MIDI keyboard or controller;", "it shows here."];

/// The label of a plugin maker the catalogue does not name.
const NO_MAKER: &str = "Other makers";

/// A MIDI input port as the Library lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    /// Whether its messages reach the app: the tick, the Settings panel's MIDI input choice.
    pub plays: bool,
    /// The name of the channel that plays its notes, when one is linked to it.
    pub channel: Option<String>,
    /// Listed only because a channel links to it: it is not plugged in.
    pub unplugged: bool,
    /// The system's loopback port ("Midi Through"), which only echoes what a program sends
    /// it: no device.
    pub loopback: bool,
    /// It sent something in the last moment ([`crate::midi::LIGHT`]).
    pub lit: bool,
    /// Where its notes go now while it plays and no channel is linked to it, as the row says
    /// it after the arrow: "selected: Kick", "preview: Upright Piano", "nothing selected".
    pub goes: Option<String>,
    /// Why the channel its notes go to plays nothing, when it cannot.
    pub silent: Option<String>,
    /// The name of the channel whose MIDI out sends to this device (its output port, matched
    /// by its stable name), when one does.
    pub out: Option<String>,
}

impl Port {
    /// What the row says after its name: where its notes go ("→ kick", "→ selected: Kick"),
    /// then which channel's MIDI out sends to it ("out: External input"), or either alone.
    pub fn arrow(&self) -> Option<String> {
        let goes = self.channel.as_ref().or(self.goes.as_ref());
        match (goes, &self.out) {
            (Some(goes), Some(out)) => Some(format!("\u{2192} {goes} \u{b7} out: {out}")),
            (Some(goes), None) => Some(format!("\u{2192} {goes}")),
            (None, Some(out)) => Some(format!("\u{2192} out: {out}")),
            (None, None) => None,
        }
    }
}

/// Whether `name` is the system's loopback port: ALSA's "Midi Through", which every Linux
/// machine lists beside the devices and which plays nothing by itself.
pub fn is_loopback(name: &str) -> bool {
    name.starts_with("Midi Through")
}

/// An instrument set the Library lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentSet {
    /// What its row and a channel made from it are called.
    pub name: String,
    /// Who made it; empty when nothing says.
    pub maker: String,
    /// Its main .sfz file.
    pub sfz: PathBuf,
    /// The memory its sounds take decoded, when its files could be measured.
    pub bytes: Option<u64>,
}

/// The instrument sets found on disk: shipped, and the user's own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sets {
    pub shipped: Vec<InstrumentSet>,
    pub mine: Vec<InstrumentSet>,
}

/// The group label over the shipped sets, and over the user's own.
pub const SHIPPED_SETS: &str = "Instrument sets";
pub const MY_SETS: &str = "My instruments";

/// The group labels over the starter kit's sounds and over the plugin instruments.
pub const STARTER_KIT: &str = "Starter kit";
pub const PLUGINS: &str = "Plugins";

/// The name a group label folds by, kept in the settings: a group's own label ("Plugins"), a
/// kind under the shipped sets after theirs ("Instrument sets/Drums"), a maker after the
/// plugins' ("Plugins/Example"), so a maker named as a kind folds alone. None for a row that
/// is no group label.
pub fn fold_key(item: &Item) -> Option<String> {
    match item {
        Item::Group(name) => Some((*name).to_owned()),
        Item::Kind(kind) => Some(format!("{SHIPPED_SETS}/{}", kind.title())),
        Item::Maker(maker) => Some(maker_key(maker)),
        _ => None,
    }
}

fn maker_key(maker: &str) -> String {
    format!("{PLUGINS}/{maker}")
}

/// The group label over "External input", and that row's name.
pub const AUDIO_INPUT: &str = "Audio input";
pub const EXTERNAL_INPUT: &str = "External input";

/// Whether the search `query` (lower case) finds "External input": its name, or its group's.
fn external_input_matches(query: &str) -> bool {
    EXTERNAL_INPUT.to_lowercase().contains(query) || AUDIO_INPUT.to_lowercase().contains(query)
}

/// What a shipped instrument set plays, as the Library sorts them, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Drums,
    Bass,
    /// Guitars: acoustic, electric, their kin (the bass guitar is Bass).
    Guitar,
    Keys,
    Pads,
    Strings,
    Mallets,
    Synth,
    /// Whatever its `set.json` does not say, or says in a word the Library does not know.
    Other,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Drums,
        Kind::Bass,
        Kind::Guitar,
        Kind::Keys,
        Kind::Pads,
        Kind::Strings,
        Kind::Mallets,
        Kind::Synth,
        Kind::Other,
    ];

    /// Its label over its sets, and the word a `set.json` `category` names it by (any case).
    pub fn title(self) -> &'static str {
        match self {
            Kind::Drums => "Drums",
            Kind::Bass => "Bass",
            Kind::Guitar => "Guitar",
            Kind::Keys => "Keys",
            Kind::Pads => "Pads",
            Kind::Strings => "Strings",
            Kind::Mallets => "Mallets",
            Kind::Synth => "Synth",
            Kind::Other => "Other",
        }
    }

    /// The kind a `set.json` `category` names, ignoring case; none for a word it does not
    /// know.
    pub fn from_category(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.title().eq_ignore_ascii_case(word.trim()))
    }
}

/// The kinds of the shipped sets, by main file. A set it does not hold is [`Kind::Other`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kinds(pub BTreeMap<PathBuf, Kind>);

impl Kinds {
    pub fn of(&self, set: &InstrumentSet) -> Kind {
        self.0.get(&set.sfz).copied().unwrap_or(Kind::Other)
    }
}

/// One row of the Library.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A section's header: its title and count, and whether its rows show.
    Header {
        section: Section,
        count: usize,
        open: bool,
    },
    /// The label over a group of instruments: "Starter kit" or "Plugins".
    Group(&'static str),
    /// The label over the shipped sets of one kind.
    Kind(Kind),
    /// A plugin maker, over its instruments.
    Maker(String),
    /// The starter kit's sound at this index.
    Sound(usize),
    /// An instrument set, shipped or the user's own.
    Set(InstrumentSet),
    /// A plugin instrument of either format.
    Plugin(Offered),
    /// Rescan, under the plugins.
    Rescan,
    /// "External input": adds an input channel.
    ExternalInput,
    /// A line of a section's empty state.
    Hint(&'static str),
    /// The Samples tree's row at this index, counted as the browser model's rows are.
    Tree(usize),
    Port(Port),
}

/// What the sections hold now.
#[derive(Debug, Clone, Default)]
pub struct Contents<'a> {
    /// What the search box holds; empty shows everything.
    pub search: &'a str,
    /// The starter kit's sound names, in order.
    pub kit: &'a [&'a str],
    /// The catalogue's plugin instruments of both formats, as it offers them.
    pub plugins: Vec<Offered>,
    /// How many rows the Samples tree shows (its matches, while searching).
    pub tree: usize,
    /// How many folders the Samples tree lists at its top.
    pub folders: usize,
    pub ports: Vec<Port>,
}

/// The Library's own state: which sections are closed and which groups folded, how far the
/// list is scrolled, and the instrument sets its last look on disk found, with the shipped
/// ones' kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    pub closed: BTreeSet<Section>,
    /// The groups folded, by [`fold_key`]: their labels show alone, except while searching.
    pub folded: BTreeSet<String>,
    /// The first row shown.
    pub top: usize,
    pub sets: Sets,
    pub kinds: Kinds,
}

impl Library {
    /// The rows, top to bottom.
    pub fn items(&self, contents: &Contents) -> Vec<Item> {
        let query = contents.search.to_lowercase();
        let searching = !query.is_empty();
        let sounds: Vec<usize> = (0..contents.kit.len())
            .filter(|&index| contents.kit[index].to_lowercase().contains(&query))
            .collect();
        let plugins: Vec<&Offered> = contents
            .plugins
            .iter()
            .filter(|offered| picker::plugin_matches(&offered.plugin, &query))
            .collect();
        let ports: Vec<&Port> = contents
            .ports
            .iter()
            .filter(|port| port.name.to_lowercase().contains(&query))
            .collect();
        let found = |sets: &[InstrumentSet]| -> Vec<InstrumentSet> {
            sets.iter()
                .filter(|set| set_matches(set, &query))
                .cloned()
                .collect()
        };
        let (shipped, mine) = (found(&self.sets.shipped), found(&self.sets.mine));
        let samples = if searching {
            contents.tree
        } else {
            contents.folders
        };
        let mut items = Vec::new();
        let instruments = sounds.len() + shipped.len() + mine.len() + plugins.len();
        // A search shows every match: folding is for browsing.
        let open = |key: &str| searching || !self.folded.contains(key);
        if self.header(Section::Instruments, instruments, &mut items) {
            if !sounds.is_empty() {
                items.push(Item::Group(STARTER_KIT));
                if open(STARTER_KIT) {
                    items.extend(sounds.into_iter().map(Item::Sound));
                }
            }
            if !shipped.is_empty() {
                items.push(Item::Group(SHIPPED_SETS));
                if open(SHIPPED_SETS) {
                    self.push_by_kind(shipped, searching, &mut items);
                }
            }
            if !mine.is_empty() {
                items.push(Item::Group(MY_SETS));
                if open(MY_SETS) {
                    items.extend(mine.into_iter().map(Item::Set));
                }
            }
            if !searching || !plugins.is_empty() {
                items.push(Item::Group(PLUGINS));
            }
            // Folded, the plugins' label stands for its makers, its empty state and Rescan.
            if open(PLUGINS) {
                if !searching && contents.plugins.is_empty() {
                    items.extend(PLUGIN_HINT.iter().map(|line| Item::Hint(line)));
                }
                for (maker, made) in by_maker(plugins) {
                    let shows = open(&maker_key(&maker));
                    items.push(Item::Maker(maker));
                    if shows {
                        items.extend(made.into_iter().cloned().map(Item::Plugin));
                    }
                }
                if !searching {
                    items.push(Item::Rescan);
                }
            }
            if external_input_matches(&query) {
                items.push(Item::Group(AUDIO_INPUT));
                if open(AUDIO_INPUT) {
                    items.push(Item::ExternalInput);
                }
            }
        }
        if self.header(Section::Samples, samples, &mut items) {
            items.extend((0..contents.tree).map(Item::Tree));
        }
        if self.header(Section::Devices, ports.len(), &mut items) {
            if !searching && contents.ports.is_empty() {
                items.extend(DEVICE_HINT.iter().map(|line| Item::Hint(line)));
            }
            items.extend(ports.into_iter().cloned().map(Item::Port));
        }
        items
    }

    /// Pushes the shipped sets `shipped` (in name order) by kind, each kind under its label;
    /// one list in name order while `searching` (the search is flat) or when they are all of
    /// one kind, which needs no label.
    fn push_by_kind(&self, shipped: Vec<InstrumentSet>, searching: bool, items: &mut Vec<Item>) {
        let mut by_kind: BTreeMap<Kind, Vec<InstrumentSet>> = BTreeMap::new();
        for set in shipped {
            by_kind.entry(self.kinds.of(&set)).or_default().push(set);
        }
        let labeled = !searching && by_kind.len() > 1;
        let mut sets: Vec<(Kind, InstrumentSet)> = by_kind
            .into_iter()
            .flat_map(|(kind, sets)| sets.into_iter().map(move |set| (kind, set)))
            .collect();
        if !labeled {
            // Back in name order, as the look on disk sorted them.
            sets.sort_by_key(|(_, set)| set.name.to_lowercase());
        }
        let mut last = None;
        for (kind, set) in sets {
            if labeled && last != Some(kind) {
                items.push(Item::Kind(kind));
                last = Some(kind);
            }
            let folded = labeled
                && fold_key(&Item::Kind(kind)).is_some_and(|key| self.folded.contains(&key));
            if !folded {
                items.push(Item::Set(set));
            }
        }
    }

    /// Pushes `section`'s header; returns whether its rows show.
    fn header(&self, section: Section, count: usize, items: &mut Vec<Item>) -> bool {
        let open = !self.closed.contains(&section);
        items.push(Item::Header {
            section,
            count,
            open,
        });
        open
    }

    /// Folds the group `key` names ([`fold_key`]), or opens it when it is folded.
    pub fn toggle_group(&mut self, key: &str) {
        if !self.folded.remove(key) {
            self.folded.insert(key.to_owned());
        }
    }

    /// Folds the group `key` names, or opens it.
    pub fn fold(&mut self, key: &str, folded: bool) {
        if folded {
            self.folded.insert(key.to_owned());
        } else {
            self.folded.remove(key);
        }
    }

    /// Opens a closed section, or closes an open one.
    pub fn toggle(&mut self, section: Section) {
        if !self.closed.remove(&section) {
            self.closed.insert(section);
        }
    }

    /// Scrolls the list the least it can to show row `row` among `visible` rows.
    pub fn reveal(&mut self, row: usize, visible: usize) {
        if row < self.top {
            self.top = row;
        } else if row >= self.top + visible.max(1) {
            self.top = row + 1 - visible.max(1);
        }
    }

    /// Scrolls the list of `total` rows by `rows` (up is negative), keeping `visible` rows'
    /// worth of it in view; 0 rows only keeps it so, after the list shrank.
    pub fn scroll(&mut self, rows: i64, visible: usize, total: usize) {
        let last = total.saturating_sub(visible.max(1));
        let top = i64::try_from(self.top)
            .unwrap_or(i64::MAX)
            .saturating_add(rows);
        self.top = usize::try_from(top.max(0)).unwrap_or(0).min(last);
    }
}

/// Whether `set` matches the lowercase `query`: by name or maker, ignoring case.
fn set_matches(set: &InstrumentSet, query: &str) -> bool {
    set.name.to_lowercase().contains(query) || set.maker.to_lowercase().contains(query)
}

/// `plugins` grouped by maker, makers sorted ignoring case (a nameless one last, as "Other
/// makers"); each maker's plugins keep their order.
fn by_maker(plugins: Vec<&Offered>) -> Vec<(String, Vec<&Offered>)> {
    let mut makers: BTreeMap<(bool, String), (String, Vec<&Offered>)> = BTreeMap::new();
    for plugin in plugins {
        let maker = plugin.plugin.vendor.trim();
        let key = (maker.is_empty(), maker.to_lowercase());
        let label = if maker.is_empty() { NO_MAKER } else { maker };
        makers
            .entry(key)
            .or_insert_with(|| (label.to_owned(), Vec::new()))
            .1
            .push(plugin);
    }
    makers.into_values().collect()
}

/// The MIDI devices to list: the ports `listed`, in order, each with its tick from `inputs` and
/// the channel `links` (port, channel name) names for it; then, sorted by name, the ports a link
/// names that are not listed, as unplugged. A link names a listed port whatever numbers the
/// system gave it ([`same_port`]).
pub fn ports(listed: &[String], inputs: &MidiInputs, links: &[(String, String)]) -> Vec<Port> {
    let channel_of = |name: &str| {
        links
            .iter()
            .find(|(port, _)| same_port(port, name))
            .map(|(_, channel)| channel.clone())
    };
    let mut gone: Vec<&(String, String)> = links
        .iter()
        .filter(|(port, _)| !listed.iter().any(|name| same_port(name, port)))
        .collect();
    gone.sort();
    let (loopbacks, devices): (Vec<&String>, Vec<&String>) =
        listed.iter().partition(|name| is_loopback(name));
    devices
        .into_iter()
        .map(|name| (name, false))
        .chain(gone.into_iter().map(|(name, _)| (name, true)))
        .chain(loopbacks.into_iter().map(|name| (name, false)))
        .map(|(name, unplugged)| Port {
            name: name.clone(),
            plays: inputs.plays(name),
            channel: channel_of(name),
            unplugged,
            loopback: is_loopback(name),
            ..Port::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_host_clap::catalogue::PluginKind;
    use dawsome_host_clap::Catalogue;

    use super::*;

    const KIT: [&str; 8] = [
        "Kick",
        "Snare",
        "Clap",
        "Hat closed",
        "Hat open",
        "Tom",
        "Bass C2",
        "Tone C4",
    ];

    /// tests/fixtures/p7_catalogue.json: instruments Analog Lead and Bell Keys by Example,
    /// Dawsome Test Sine by Dawsome; Dawsome Test Gain is an effect; broken.clap failed.
    fn catalogue() -> Catalogue {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p7_catalogue.json");
        Catalogue::load(&path).expect("the catalogue loads")
    }

    fn port(name: &str, plays: bool, channel: Option<&str>, unplugged: bool) -> Port {
        Port {
            loopback: is_loopback(name),
            name: name.to_owned(),
            plays,
            channel: channel.map(str::to_owned),
            unplugged,
            ..Port::default()
        }
    }

    fn links() -> Vec<(String, String)> {
        vec![
            ("Port A".to_owned(), "kick".to_owned()),
            ("Port C".to_owned(), "keys".to_owned()),
        ]
    }

    /// Ports A and B plugged in, A playing; "kick" linked to A and "keys" to C, not plugged in.
    fn three_ports() -> Vec<Port> {
        let listed = ["Port A".to_owned(), "Port B".to_owned()];
        let inputs = MidiInputs::Named(BTreeSet::from(["Port A".to_owned()]));
        ports(&listed, &inputs, &links())
    }

    fn plugin(catalogue: &Catalogue, name: &str) -> Item {
        let found = catalogue
            .offered(PluginKind::Instrument)
            .into_iter()
            .find(|plugin| plugin.plugin.name == name)
            .expect("in the catalogue");
        Item::Plugin(found)
    }

    fn header(section: Section, count: usize, open: bool) -> Item {
        Item::Header {
            section,
            count,
            open,
        }
    }

    // By hand: 8 kit sounds and the catalogue's 3 instruments make 11 (the effect is not one,
    // nor is the external input, last under its own group); makers sort Dawsome before
    // Example; one folder above the tree's 3 rows; A and B listed and C, which "keys" links to,
    // unplugged after them: 3 devices.
    #[test]
    fn the_sections_list_the_kit_the_plugins_by_maker_the_tree_and_the_ports() {
        let catalogue = catalogue();
        let contents = Contents {
            kit: &KIT,
            plugins: catalogue.offered(PluginKind::Instrument),
            tree: 3,
            folders: 1,
            ports: three_ports(),
            ..Contents::default()
        };
        let mut expected = vec![
            header(Section::Instruments, 11, true),
            Item::Group("Starter kit"),
        ];
        expected.extend((0..8).map(Item::Sound));
        expected.extend([
            Item::Group("Plugins"),
            Item::Maker("Dawsome".to_owned()),
            plugin(&catalogue, "Dawsome Test Sine"),
            Item::Maker("Example".to_owned()),
            plugin(&catalogue, "Analog Lead"),
            plugin(&catalogue, "Bell Keys"),
            Item::Rescan,
            Item::Group(AUDIO_INPUT),
            Item::ExternalInput,
            header(Section::Samples, 1, true),
            Item::Tree(0),
            Item::Tree(1),
            Item::Tree(2),
            header(Section::Devices, 3, true),
            Item::Port(port("Port A", true, Some("kick"), false)),
            Item::Port(port("Port B", false, None, false)),
            Item::Port(port("Port C", false, Some("keys"), true)),
        ]);
        assert_eq!(Library::default().items(&contents), expected);
    }

    // One search over all three: "TONE" finds the kit's Tone C4 (any case) and whatever the
    // tree's own search found (here 1 file), the Samples count becoming its matches; "bell"
    // finds Bell Keys under its maker; "port b" one device. A section with no match keeps its
    // header at 0, and no hint, Rescan or empty group shows while searching.
    #[test]
    fn the_search_filters_every_section_at_once() {
        let catalogue = catalogue();
        let items = |search: &str, tree: usize| {
            Library::default().items(&Contents {
                search,
                kit: &KIT,
                plugins: catalogue.offered(PluginKind::Instrument),
                tree,
                folders: 1,
                ports: three_ports(),
            })
        };
        assert_eq!(
            items("TONE", 1),
            [
                header(Section::Instruments, 1, true),
                Item::Group("Starter kit"),
                Item::Sound(7),
                header(Section::Samples, 1, true),
                Item::Tree(0),
                header(Section::Devices, 0, true),
            ]
        );
        assert_eq!(
            items("bell", 0),
            [
                header(Section::Instruments, 1, true),
                Item::Group("Plugins"),
                Item::Maker("Example".to_owned()),
                plugin(&catalogue, "Bell Keys"),
                header(Section::Samples, 0, true),
                header(Section::Devices, 0, true),
            ]
        );
        assert_eq!(
            items("port b", 0),
            [
                header(Section::Instruments, 0, true),
                header(Section::Samples, 0, true),
                header(Section::Devices, 1, true),
                Item::Port(port("Port B", false, None, false)),
            ]
        );
    }

    // The external input is found by its name or its group's, alone among the instruments, and
    // is not counted among them; a search for anything else leaves it out.
    #[test]
    fn the_external_input_is_searched_by_its_name_and_not_counted() {
        let contents = |search| Contents {
            search,
            kit: &KIT,
            ..Contents::default()
        };
        assert_eq!(
            Library::default().items(&contents("INPUT")),
            [
                header(Section::Instruments, 0, true),
                Item::Group(AUDIO_INPUT),
                Item::ExternalInput,
                header(Section::Samples, 0, true),
                header(Section::Devices, 0, true),
            ]
        );
        let audio = Library::default().items(&contents("audio"));
        assert!(audio.contains(&Item::ExternalInput));
        let kick = Library::default().items(&contents("kick"));
        assert!(!kick.contains(&Item::ExternalInput));
    }

    // No plugin instruments and no ports: their hints show, the Plugins group and Rescan stay
    // (the external input after them); a closed section keeps its header and count and hides
    // its rows.
    #[test]
    fn empty_sections_hint_and_closed_ones_keep_their_count() {
        let contents = Contents {
            kit: &KIT,
            tree: 2,
            folders: 2,
            ..Contents::default()
        };
        let mut library = Library::default();
        let mut expected = vec![
            header(Section::Instruments, 8, true),
            Item::Group("Starter kit"),
        ];
        expected.extend((0..8).map(Item::Sound));
        expected.push(Item::Group("Plugins"));
        expected.extend(PLUGIN_HINT.iter().map(|line| Item::Hint(line)));
        expected.extend([
            Item::Rescan,
            Item::Group(AUDIO_INPUT),
            Item::ExternalInput,
            header(Section::Samples, 2, true),
            Item::Tree(0),
            Item::Tree(1),
            header(Section::Devices, 0, true),
        ]);
        expected.extend(DEVICE_HINT.iter().map(|line| Item::Hint(line)));
        assert_eq!(library.items(&contents), expected);
        library.toggle(Section::Instruments);
        library.toggle(Section::Devices);
        assert_eq!(
            library.items(&contents),
            [
                header(Section::Instruments, 8, false),
                header(Section::Samples, 2, true),
                Item::Tree(0),
                Item::Tree(1),
                header(Section::Devices, 0, false),
            ]
        );
        library.toggle(Section::Devices);
        assert!(!library.closed.contains(&Section::Devices), "opens again");
    }

    // A port a channel links to stays listed while it is unplugged, dim with its channel; once
    // no channel links to it, it is gone. A plugged-in port lists with its link and its tick.
    #[test]
    fn a_linked_port_that_vanished_stays_listed_as_unplugged() {
        let inputs = MidiInputs::All;
        let listed = ["Port A".to_owned()];
        assert_eq!(
            ports(&listed, &inputs, &links()),
            [
                port("Port A", true, Some("kick"), false),
                port("Port C", true, Some("keys"), true),
            ]
        );
        let kick_only = &links()[..1];
        assert_eq!(
            ports(&listed, &inputs, kick_only),
            [port("Port A", true, Some("kick"), false)]
        );
        assert_eq!(ports(&[], &inputs, &[]), []);
    }

    // The system's loopback ("Midi Through", as ALSA lists it on every Linux
    // machine beside a keyboard) lists last, after the devices and the unplugged links, marked
    // a loopback and tickable as any port ("a keyboard shows as two devices", 2026-10-23).
    #[test]
    fn the_midi_through_loopback_lists_last() {
        let inputs = MidiInputs::Named(BTreeSet::from([
            "Midi Through:Midi Through Port-0".to_owned()
        ]));
        let listed = [
            "Midi Through:Midi Through Port-0 14:0".to_owned(),
            "Oriole:Oriole MIDI 1 28:0".to_owned(),
        ];
        let listed_ports = ports(&listed, &inputs, &links()[1..]);
        let names: Vec<&str> = listed_ports.iter().map(|port| port.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Oriole:Oriole MIDI 1 28:0",
                "Port C",
                "Midi Through:Midi Through Port-0 14:0"
            ]
        );
        let flags: Vec<(bool, bool, bool)> = listed_ports
            .iter()
            .map(|port| (port.loopback, port.unplugged, port.plays))
            .collect();
        assert_eq!(
            flags,
            [
                (false, false, false),
                (false, true, false),
                (true, false, true)
            ]
        );
        assert!(is_loopback("Midi Through Port-0") && !is_loopback("Oriole MIDI 1"));
    }

    // A row kept in view: one above the top becomes the top, one below the last shown becomes
    // the last shown, and one in view moves nothing.
    #[test]
    fn a_row_kept_in_view_scrolls_the_least_it_can() {
        let mut library = Library {
            top: 5,
            ..Library::default()
        };
        library.reveal(7, 10);
        assert_eq!(library.top, 5, "in view");
        library.reveal(20, 10);
        assert_eq!(library.top, 11, "row 20 is the last of rows 11 to 20");
        library.reveal(3, 10);
        assert_eq!(library.top, 3);
        library.reveal(0, 0);
        assert_eq!(library.top, 0, "no room: the row is the top");
    }

    // The list scrolls within its length: a screenful stays in view, and a list that shrank
    // under the top scrolls back to its last screenful.
    #[test]
    fn the_list_scrolls_within_its_rows() {
        let mut library = Library::default();
        library.scroll(3, 10, 25);
        assert_eq!(library.top, 3);
        library.scroll(30, 10, 25);
        assert_eq!(library.top, 15);
        library.scroll(-100, 10, 25);
        assert_eq!(library.top, 0);
        library.scroll(15, 10, 25);
        library.scroll(0, 10, 12);
        assert_eq!(library.top, 2, "shrunk to 12 rows");
        library.scroll(0, 10, 4);
        assert_eq!(library.top, 0, "fits whole");
    }

    fn set(name: &str, maker: &str) -> InstrumentSet {
        InstrumentSet {
            name: name.to_owned(),
            maker: maker.to_owned(),
            sfz: PathBuf::from(format!("{name}.sfz")),
            bytes: None,
        }
    }

    // B6: the shipped sets by kind, drums first, each kind under its label in name order; the
    // search lists them flat in name order; sets of one kind take no label, and the user's own
    // are never sorted by kind.
    #[test]
    fn shipped_sets_list_by_kind_and_the_search_lists_them_flat() {
        // In name order, as the look on disk sorts them.
        let shipped = vec![
            set("Electric Bass", "FreePats"),
            set("Electric Piano", "FreePats"),
            set("Electronic Drums", "Someone"),
            set("Lead", "Dawsome"),
            set("Odd", "Someone"),
            set("Upright Piano", "FreePats"),
        ];
        let kinds = Kinds(
            [
                ("Electric Bass", Kind::Bass),
                ("Electric Piano", Kind::Keys),
                ("Electronic Drums", Kind::Drums),
                ("Lead", Kind::Synth),
                ("Upright Piano", Kind::Keys),
            ]
            .into_iter()
            .map(|(name, kind)| (PathBuf::from(format!("{name}.sfz")), kind))
            .collect(),
        );
        let library = Library {
            sets: Sets {
                shipped: shipped.clone(),
                mine: vec![set("My pad", "")],
            },
            kinds,
            ..Library::default()
        };
        let items = library.items(&Contents::default());
        let set_item = |name: &str| Item::Set(set(name, maker_of(&shipped, name)));
        assert_eq!(
            items[1..13],
            [
                Item::Group(SHIPPED_SETS),
                Item::Kind(Kind::Drums),
                set_item("Electronic Drums"),
                Item::Kind(Kind::Bass),
                set_item("Electric Bass"),
                Item::Kind(Kind::Keys),
                set_item("Electric Piano"),
                set_item("Upright Piano"),
                Item::Kind(Kind::Synth),
                set_item("Lead"),
                Item::Kind(Kind::Other),
                set_item("Odd"),
            ]
        );
        assert_eq!(
            items[13..15],
            [Item::Group(MY_SETS), Item::Set(set("My pad", ""))]
        );
        let searched = library.items(&Contents {
            search: "e",
            ..Contents::default()
        });
        // "e" is in every shipped set's name or maker, and not in "My pad".
        let names: Vec<&str> = searched
            .iter()
            .filter_map(|item| match item {
                Item::Set(set) => Some(set.name.as_str()),
                Item::Kind(_) => Some("a kind's label"),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            [
                "Electric Bass",
                "Electric Piano",
                "Electronic Drums",
                "Lead",
                "Odd",
                "Upright Piano"
            ]
        );
        let one_kind = Library {
            sets: Sets {
                shipped: vec![
                    set("Electric Piano", "FreePats"),
                    set("Upright Piano", "FreePats"),
                ],
                mine: Vec::new(),
            },
            kinds: library.kinds.clone(),
            ..Library::default()
        };
        assert_eq!(
            one_kind.items(&Contents::default())[1..4],
            [
                Item::Group(SHIPPED_SETS),
                Item::Set(set("Electric Piano", "FreePats")),
                Item::Set(set("Upright Piano", "FreePats")),
            ]
        );
        assert_eq!(Kind::from_category(" keys "), Some(Kind::Keys));
        assert_eq!(Kind::from_category("Woodwinds"), None);
    }

    // The groups inside Instruments fold on their labels. Folded,
    // "Starter kit" shows its label alone; "Keys" under the shipped sets hides its two sets and
    // leaves the other kinds; "Instrument sets" hides every kind and set under it; a folded
    // maker hides its plugins, the other maker keeps them; "Plugins" folded hides its makers,
    // plugins and Rescan; "Audio input" hides External input. The section's count is the
    // same folded or not, and a search shows every match whatever is folded. Folding twice is
    // opening again.
    #[test]
    fn the_instruments_groups_fold_and_a_search_ignores_it() {
        let catalogue = catalogue();
        let shipped = vec![
            set("Electric Bass", "FreePats"),
            set("Electric Piano", "FreePats"),
            set("Upright Piano", "FreePats"),
        ];
        let kinds = Kinds(
            [
                ("Electric Bass", Kind::Bass),
                ("Electric Piano", Kind::Keys),
                ("Upright Piano", Kind::Keys),
            ]
            .into_iter()
            .map(|(name, kind)| (PathBuf::from(format!("{name}.sfz")), kind))
            .collect(),
        );
        let mut library = Library {
            sets: Sets {
                shipped: shipped.clone(),
                mine: Vec::new(),
            },
            kinds,
            ..Library::default()
        };
        let contents = Contents {
            kit: &KIT[..2],
            plugins: catalogue.offered(PluginKind::Instrument),
            ..Contents::default()
        };
        let set_item = |name: &str| Item::Set(set(name, maker_of(&shipped, name)));
        let unfolded = library.items(&contents);
        assert_eq!(unfolded[0], header(Section::Instruments, 8, true));
        for key in [
            STARTER_KIT,
            "Instrument sets/Keys",
            "Plugins/Example",
            AUDIO_INPUT,
        ] {
            library.toggle_group(key);
        }
        let folded = library.items(&contents);
        assert_eq!(
            folded,
            [
                header(Section::Instruments, 8, true),
                Item::Group(STARTER_KIT),
                Item::Group(SHIPPED_SETS),
                Item::Kind(Kind::Bass),
                set_item("Electric Bass"),
                Item::Kind(Kind::Keys),
                Item::Group(PLUGINS),
                Item::Maker("Dawsome".to_owned()),
                plugin(&catalogue, "Dawsome Test Sine"),
                Item::Maker("Example".to_owned()),
                Item::Rescan,
                Item::Group(AUDIO_INPUT),
                header(Section::Samples, 0, true),
                header(Section::Devices, 0, true),
                Item::Hint(DEVICE_HINT[0]),
                Item::Hint(DEVICE_HINT[1]),
            ]
        );
        library.fold(SHIPPED_SETS, true);
        library.fold(PLUGINS, true);
        let groups = library.items(&contents);
        assert_eq!(
            groups[..6],
            [
                header(Section::Instruments, 8, true),
                Item::Group(STARTER_KIT),
                Item::Group(SHIPPED_SETS),
                Item::Group(PLUGINS),
                Item::Group(AUDIO_INPUT),
                header(Section::Samples, 0, true),
            ]
        );
        let searched = library.items(&Contents {
            search: "e",
            ..contents.clone()
        });
        let shown = |items: &[Item], wanted: &Item| items.contains(wanted);
        for wanted in [
            set_item("Electric Piano"),
            set_item("Upright Piano"),
            plugin(&catalogue, "Bell Keys"),
            Item::ExternalInput,
        ] {
            assert!(shown(&searched, &wanted), "a search shows {wanted:?}");
        }
        for key in [
            STARTER_KIT,
            "Instrument sets/Keys",
            "Plugins/Example",
            AUDIO_INPUT,
            SHIPPED_SETS,
            PLUGINS,
        ] {
            library.toggle_group(key);
        }
        assert_eq!(library.items(&contents), unfolded, "every group open again");
        assert_eq!(
            fold_key(&Item::Kind(Kind::Drums)).as_deref(),
            Some("Instrument sets/Drums")
        );
        assert_eq!(
            fold_key(&Item::Maker("Drums".to_owned())).as_deref(),
            Some("Plugins/Drums")
        );
        assert_eq!(fold_key(&Item::Group(MY_SETS)).as_deref(), Some(MY_SETS));
        assert_eq!(fold_key(&Item::Rescan), None);
    }

    fn maker_of<'a>(sets: &'a [InstrumentSet], name: &str) -> &'a str {
        sets.iter()
            .find(|set| set.name == name)
            .map_or("", |set| set.maker.as_str())
    }

    // The sets sit under the kit, shipped then the user's own, each group only when it has a
    // set, and count among the instruments; the search finds them by name or maker.
    #[test]
    fn instrument_sets_list_under_the_kit_and_are_searched_by_name_or_maker() {
        let library = Library {
            sets: Sets {
                shipped: vec![set("Piano", "Dawsome"), set("Strings", "Dawsome")],
                mine: vec![set("My pad", "")],
            },
            ..Library::default()
        };
        let contents = Contents {
            kit: &KIT[..1],
            ..Contents::default()
        };
        let items = library.items(&contents);
        assert_eq!(
            items[..7],
            [
                header(Section::Instruments, 4, true),
                Item::Group("Starter kit"),
                Item::Sound(0),
                Item::Group(SHIPPED_SETS),
                Item::Set(set("Piano", "Dawsome")),
                Item::Set(set("Strings", "Dawsome")),
                Item::Group(MY_SETS),
            ]
        );
        assert_eq!(items[7], Item::Set(set("My pad", "")));
        assert_eq!(items[8], Item::Group("Plugins"));
        let searched = library.items(&Contents {
            search: "dawsome",
            ..contents.clone()
        });
        assert_eq!(
            searched[..4],
            [
                header(Section::Instruments, 2, true),
                Item::Group(SHIPPED_SETS),
                Item::Set(set("Piano", "Dawsome")),
                Item::Set(set("Strings", "Dawsome")),
            ]
        );
        let only_mine = Library {
            sets: Sets {
                shipped: Vec::new(),
                mine: vec![set("My pad", "")],
            },
            ..Library::default()
        };
        let items = only_mine.items(&Contents::default());
        assert_eq!(
            items[..3],
            [
                header(Section::Instruments, 1, true),
                Item::Group(MY_SETS),
                Item::Set(set("My pad", "")),
            ]
        );
    }
}
