//! The Help window's guide: its text, built into the program from `assets/help/guide.md` (the
//! one source of the words), read into sections ([`Guide`]), and its pictures, which are not
//! built in: they are read from the help folder installed with the program ([`folder`]) when a
//! section showing them opens, so the program stays small, and a picture that is missing or
//! unreadable leaves its caption alone ([`load_picture`]).
//!
//! The guide is plain Markdown of a few kinds of line, so it reads as a document in any editor
//! and needs no parser beyond this one: `# ` starts a section (an entry of the contents), `## ` a
//! heading inside it, `- ` a point of a list, `![caption](file.png)` alone on its line a picture
//! from the help folder; other lines are paragraphs, a blank line between two. A line indented
//! by two spaces carries on the point above it. `**words**` are bold. Lines before the first
//! section are notes for whoever edits it, never shown.

use std::path::{Path, PathBuf};

/// The guide's text, as `assets/help/guide.md` holds it.
pub const GUIDE: &str = include_str!("../../../assets/help/guide.md");

/// The variable naming the help folder in place of the one found beside the program.
pub const FOLDER_VAR: &str = "DAWSOME_HELP_DIR";

/// The help folder's name beside the program and under the install prefix's
/// `share/dawsome/`.
const HELP: &str = "help";

/// The guide in sections, in its order.
#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    pub sections: Vec<Section>,
}

/// A section of the guide: an entry of the contents and the page it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub blocks: Vec<Block>,
}

/// A piece of a section's page, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A heading inside the section (`## `).
    Heading(Text),
    Paragraph(Text),
    /// A point of a list (`- `).
    Point(Text),
    /// A picture of the help folder, by its file name, and the words under it.
    Picture {
        file: String,
        caption: Text,
    },
}

/// Words, in runs plain or bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text(pub Vec<Run>);

/// A run of words: bold or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub words: String,
    pub bold: bool,
}

impl Text {
    /// `line` read for `**bold**` runs; an unpaired `**` is kept as written.
    fn of(line: &str) -> Self {
        let mut runs = Vec::new();
        let mut rest = line;
        let mut bold = false;
        while let Some(at) = rest.find("**") {
            let closes = bold || rest[at + 2..].contains("**");
            if !closes {
                break;
            }
            if at > 0 {
                runs.push(Run {
                    words: rest[..at].to_owned(),
                    bold,
                });
            }
            bold = !bold;
            rest = &rest[at + 2..];
        }
        if !rest.is_empty() {
            runs.push(Run {
                words: rest.to_owned(),
                bold,
            });
        }
        Self(runs)
    }

    /// The words without their marks.
    pub fn plain(&self) -> String {
        self.0.iter().map(|run| run.words.as_str()).collect()
    }
}

impl Guide {
    /// The guide the program carries.
    pub fn shipped() -> Self {
        Self::parse(GUIDE)
    }

    /// Reads `text` as the module's notes describe: its sections and their blocks.
    pub fn parse(text: &str) -> Self {
        let mut sections: Vec<Section> = Vec::new();
        // The paragraph or point being read, and its lines so far.
        let mut open: Option<(bool, String)> = None;
        let close = |open: &mut Option<(bool, String)>, sections: &mut Vec<Section>| {
            let Some((point, words)) = open.take() else {
                return;
            };
            if let Some(section) = sections.last_mut() {
                let text = Text::of(&words);
                section.blocks.push(if point {
                    Block::Point(text)
                } else {
                    Block::Paragraph(text)
                });
            }
        };
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(title) = line.strip_prefix("# ") {
                close(&mut open, &mut sections);
                sections.push(Section {
                    title: title.trim().to_owned(),
                    blocks: Vec::new(),
                });
                continue;
            }
            if sections.is_empty() {
                continue;
            }
            if trimmed.is_empty() {
                close(&mut open, &mut sections);
                continue;
            }
            if let Some(heading) = line.strip_prefix("## ") {
                close(&mut open, &mut sections);
                let section = sections.last_mut().expect("a section is open");
                section
                    .blocks
                    .push(Block::Heading(Text::of(heading.trim())));
                continue;
            }
            if let Some((caption, file)) = picture(trimmed) {
                close(&mut open, &mut sections);
                let section = sections.last_mut().expect("a section is open");
                section.blocks.push(Block::Picture {
                    file: file.to_owned(),
                    caption: Text::of(caption),
                });
                continue;
            }
            if let Some(point) = line.strip_prefix("- ") {
                close(&mut open, &mut sections);
                open = Some((true, point.trim().to_owned()));
                continue;
            }
            match &mut open {
                // A line indented under a point carries it on; any other line carries on the
                // paragraph, as Markdown joins them.
                Some((true, words)) if line.starts_with("  ") => {
                    words.push(' ');
                    words.push_str(trimmed);
                }
                Some((false, words)) => {
                    words.push(' ');
                    words.push_str(trimmed);
                }
                _ => {
                    close(&mut open, &mut sections);
                    open = Some((false, trimmed.to_owned()));
                }
            }
        }
        close(&mut open, &mut sections);
        Self { sections }
    }

    /// Every picture the guide names, in its order, each once.
    pub fn pictures(&self) -> Vec<&str> {
        let mut files: Vec<&str> = Vec::new();
        for section in &self.sections {
            for block in &section.blocks {
                if let Block::Picture { file, .. } = block {
                    if !files.contains(&file.as_str()) {
                        files.push(file);
                    }
                }
            }
        }
        files
    }
}

/// `![caption](file)`, alone on its line: the caption and the file.
fn picture(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("![")?;
    let (caption, rest) = rest.split_once("](")?;
    let file = rest.strip_suffix(')')?;
    (!file.is_empty() && !file.contains(['/', '\\', '('])).then_some((caption, file))
}

/// The help folder: `$DAWSOME_HELP_DIR` when that is an absolute path, else the one installed
/// with this program ([`installed_beside`]), else, for a run from the build folder, the
/// repository's `assets/help` when it is still there.
pub fn folder() -> Option<PathBuf> {
    std::env::var_os(FOLDER_VAR)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| installed_beside(&std::env::current_exe().ok()?))
        .or_else(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/help");
            source.is_dir().then_some(source)
        })
}

/// The help folder of the program at `program`: `help/` beside it (a Windows install and the
/// release zips), else, on other systems, `share/dawsome/help` under the prefix it is installed
/// in (`<prefix>/bin/dawsome-app`, as scripts/install.sh puts it); `None` when neither is a
/// folder. The instrument sets are found the same way (`dawsome_media::paths`).
pub fn installed_beside(program: &Path) -> Option<PathBuf> {
    let dir = program.parent()?;
    let beside = dir.join(HELP);
    if beside.is_dir() {
        return Some(beside);
    }
    if cfg!(windows) {
        return None;
    }
    let shared = dir.parent()?.join("share/dawsome").join(HELP);
    shared.is_dir().then_some(shared)
}

/// What `dawsome-app --list-help` prints: the help folder (or that none was found), then a line
/// for each picture `guide` names, `found` when it is a file there, else `missing`, a tab apart.
pub fn listing(folder: Option<&Path>, guide: &Guide) -> String {
    let mut lines = match folder {
        Some(folder) => format!("folder\t{}\n", folder.display()),
        None => "folder\tnone\n".to_owned(),
    };
    for file in guide.pictures() {
        let found = folder.is_some_and(|folder| folder.join(file).is_file());
        let state = if found { "found" } else { "missing" };
        lines.push_str(&format!("picture\t{file}\t{state}\n"));
    }
    lines
}

/// A picture's pixels: width, height and 8-bit RGBA, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The picture `file` of the help folder `folder`, decoded; `None` when it is not there or is
/// no PNG this can read (the page then shows its caption alone).
pub fn load_picture(folder: Option<&Path>, file: &str) -> Option<Picture> {
    let bytes = std::fs::read(folder?.join(file)).ok()?;
    decode(&bytes)
}

/// A PNG of any 8- or 16-bit color type as 8-bit RGBA.
fn decode(bytes: &[u8]) -> Option<Picture> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(frame.buffer_size());
    let pixels = (frame.width as usize).checked_mul(frame.height as usize)?;
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|ga| [ga[0], ga[0], ga[0], ga[1]])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    (rgba.len() == pixels * 4).then_some(Picture {
        width: frame.width,
        height: frame.height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(words: &str) -> Text {
        Text(vec![Run {
            words: words.to_owned(),
            bold: false,
        }])
    }

    // The kinds of line, written by hand: notes before the first section are left out; a
    // section's heading, paragraphs joined across lines, points carried on by indented lines,
    // a picture alone on its line, bold runs.
    #[test]
    fn the_guide_reads_sections_headings_paragraphs_points_and_pictures() {
        let text = "\
Notes for the editor, never shown.

# One
First line
second line.

## A heading
- a point
  carried on
- **Space** plays
![The rack, ringed](rack.png)

# Two
Last.";
        let guide = Guide::parse(text);
        assert_eq!(guide.sections.len(), 2);
        assert_eq!(guide.sections[0].title, "One");
        assert_eq!(
            guide.sections[0].blocks,
            [
                Block::Paragraph(plain("First line second line.")),
                Block::Heading(plain("A heading")),
                Block::Point(plain("a point carried on")),
                Block::Point(Text(vec![
                    Run {
                        words: "Space".to_owned(),
                        bold: true
                    },
                    Run {
                        words: " plays".to_owned(),
                        bold: false
                    },
                ])),
                Block::Picture {
                    file: "rack.png".to_owned(),
                    caption: plain("The rack, ringed"),
                },
            ]
        );
        assert_eq!(guide.sections[1].blocks, [Block::Paragraph(plain("Last."))]);
        assert_eq!(guide.pictures(), ["rack.png"]);
    }

    // An unpaired ** stays as written; a picture naming a path or nothing is no picture.
    #[test]
    fn odd_marks_stay_words() {
        assert_eq!(Text::of("a ** b").plain(), "a ** b");
        assert_eq!(Text::of("**a** and **b").plain(), "a and **b");
        assert_eq!(picture("![x](a.png)"), Some(("x", "a.png")));
        assert_eq!(picture("![x](../a.png)"), None);
        assert_eq!(picture("![x]()"), None);
        assert_eq!(picture("see ![x](a.png)"), None);
    }

    // The help folder installed beside the program: help/ beside it, else (Linux)
    // share/dawsome/help under its prefix; neither, none.
    #[test]
    fn the_help_folder_is_found_beside_the_program_or_under_its_prefix() {
        let root = std::env::temp_dir().join(format!("dawsome-help-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("made");
        let program = bin.join("dawsome-app");
        assert_eq!(installed_beside(&program), None);
        let shared = root.join("share/dawsome/help");
        std::fs::create_dir_all(&shared).expect("made");
        if cfg!(windows) {
            assert_eq!(installed_beside(&program), None);
        } else {
            assert_eq!(installed_beside(&program), Some(shared));
        }
        std::fs::create_dir_all(bin.join("help")).expect("made");
        assert_eq!(installed_beside(&program), Some(bin.join("help")));
        std::fs::remove_dir_all(&root).expect("removed");
    }

    // `--list-help`: the folder, then each picture found or missing; no folder, all missing.
    #[test]
    fn the_listing_names_the_folder_and_each_picture() {
        let guide = Guide::parse("# One\n![a](a.png)\n![b](b.png)\n![a again](a.png)");
        let dir = std::env::temp_dir().join(format!("dawsome-help-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("made");
        std::fs::write(dir.join("a.png"), b"").expect("written");
        assert_eq!(
            listing(Some(&dir), &guide),
            format!(
                "folder\t{}\npicture\ta.png\tfound\npicture\tb.png\tmissing\n",
                dir.display()
            )
        );
        assert_eq!(
            listing(None, &guide),
            "folder\tnone\npicture\ta.png\tmissing\npicture\tb.png\tmissing\n"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // A picture is read whatever PNG color type it was saved in (Pillow writes RGB or RGBA);
    // a file that is missing or no PNG gives none, never a panic.
    #[test]
    fn a_picture_decodes_to_rgba_and_a_bad_one_to_nothing() {
        let encode = |color: png::ColorType, data: &[u8]| {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(data).expect("pixels");
            writer.finish().expect("written");
            bytes
        };
        let rgb = decode(&encode(png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6])).expect("decodes");
        assert_eq!((rgb.width, rgb.height), (2, 1));
        assert_eq!(rgb.rgba, [1, 2, 3, 255, 4, 5, 6, 255]);
        let rgba = decode(&encode(png::ColorType::Rgba, &[1, 2, 3, 4, 5, 6, 7, 8])).expect("ok");
        assert_eq!(rgba.rgba, [1, 2, 3, 4, 5, 6, 7, 8]);
        let gray = decode(&encode(png::ColorType::Grayscale, &[9, 10])).expect("decodes");
        assert_eq!(gray.rgba, [9, 9, 9, 255, 10, 10, 10, 255]);
        assert_eq!(decode(b"not a png"), None);
        let dir = std::env::temp_dir();
        assert_eq!(
            load_picture(Some(&dir), "no-such-picture-dawsome.png"),
            None
        );
        assert_eq!(load_picture(None, "screen.png"), None);
    }
}
