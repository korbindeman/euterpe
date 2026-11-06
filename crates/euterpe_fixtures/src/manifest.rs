//! The pinned fixture manifest.
//!
//! [`ARCHIVES`] lists every upstream download, pinned by size and SHA-256.
//! [`EXCERPTS`] lists the short clips cut from them and committed to
//! `fixtures/committed/`. Both are plain data so tests, the fetch tool and
//! agents all read the same source of truth.
//!
//! All sources are CC BY 4.0. Credits are in `fixtures/ATTRIBUTION.md`.

/// One upstream dataset.
#[derive(Clone, Copy, Debug)]
pub struct Dataset {
    /// Folder name under `fixtures/cache/`.
    pub id: &'static str,
    pub title: &'static str,
    /// The Zenodo record, for humans and for the license.
    pub record_url: &'static str,
    pub license: &'static str,
}

pub const GUITAR_TECHS: Dataset = Dataset {
    id: "guitar-techs",
    title: "Guitar-TECHS: electric guitar DI + amp mic, techniques, chords, scales, music",
    record_url: "https://zenodo.org/records/14963133",
    license: "CC-BY-4.0",
};

pub const EGFXSET: Dataset = Dataset {
    id: "egfxset",
    title: "EGFxSet: every Strat note at 5 pickup positions, clean and through hardware pedals",
    record_url: "https://zenodo.org/records/7044411",
    license: "CC-BY-4.0",
};

pub const EGDB_BIASFX2: Dataset = Dataset {
    id: "egdb-biasfx2",
    title: "EGDB rendered through random BIAS FX2 presets (wet only; dry is EGDB audio_DI)",
    record_url: "https://zenodo.org/records/12674910",
    license: "CC-BY-4.0",
};

pub const DATASETS: &[Dataset] = &[GUITAR_TECHS, EGFXSET, EGDB_BIASFX2];

/// One downloadable file, pinned by size and hash.
#[derive(Clone, Copy, Debug)]
pub struct Archive {
    pub dataset: Dataset,
    /// File name on Zenodo and in the cache.
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

impl Archive {
    pub fn url(&self) -> String {
        format!("{}/files/{}?download=1", self.dataset.record_url, self.file)
    }

    /// Folder the archive unpacks into, relative to the dataset's cache dir:
    /// the file name without `.zip`. `None` for files that aren't zips.
    pub fn extract_dir(&self) -> Option<&'static str> {
        self.file.strip_suffix(".zip")
    }
}

const fn a(dataset: Dataset, file: &'static str, bytes: u64, sha256: &'static str) -> Archive {
    Archive {
        dataset,
        file,
        bytes,
        sha256,
    }
}

/// Every upstream file. Hashes were taken on 2026-11-06 and the sizes match
/// the Zenodo records (md5 also verified against Zenodo at the time).
pub const ARCHIVES: &[Archive] = &[
    a(
        GUITAR_TECHS,
        "P1_chords.zip",
        981_741_162,
        "de4aa76ef4b86ce981496161b741dc39bd22dec5250da351e5e68a44da249326",
    ),
    a(
        GUITAR_TECHS,
        "P1_scales.zip",
        453_349_723,
        "79d7e9d148820867a9095697c9521e40a4b11c198384edfce1951de7219b3509",
    ),
    a(
        GUITAR_TECHS,
        "P1_singlenotes.zip",
        108_626_613,
        "130592ae5555476ea8e4070c0f3421794ef8b5e252dfa780745d07eedd0eb4a4",
    ),
    a(
        GUITAR_TECHS,
        "P1_techniques.zip",
        326_280_863,
        "1e4b80a464182d345e129f3e1158b6c05690c60b5f9be4bde3fb26f23263236e",
    ),
    a(
        GUITAR_TECHS,
        "P2_chords.zip",
        1_150_819_056,
        "9d4a46261cc840d6a66412ad0ebffcfba1fbce579bec0205127190b7bd7a4bce",
    ),
    a(
        GUITAR_TECHS,
        "P2_scales.zip",
        471_254_783,
        "d5efc7134764bd8124a712fd301d020d143a6e589eb8e1f2e0ddcbf94524ff17",
    ),
    a(
        GUITAR_TECHS,
        "P2_singlenotes.zip",
        116_133_457,
        "d6b54e40d22113d6c0a663165cb2af63735897a35bb45fc6d0ed49c944b548d9",
    ),
    a(
        GUITAR_TECHS,
        "P2_techniques.zip",
        395_839_610,
        "05fc065c010add9e5348095d7198fdc45b967c657e3e12ef8afdb74808371816",
    ),
    a(
        GUITAR_TECHS,
        "P3_music.zip",
        129_505_089,
        "033489e22600751fb5a1633e7d856b901c6782e0486fa02135e830780d9dbfe2",
    ),
    a(
        EGFXSET,
        "Clean.zip",
        431_036_829,
        "baa01d1d040c044eef81453e2c96c560722ed8e26736aff52a9490bfd94d3aaa",
    ),
    a(
        EGFXSET,
        "TubeScreamer.zip",
        471_923_566,
        "fb099f2e1a34821a29813466c5d2b10af6c18257221b04e1d63c8fc3d17d9ff1",
    ),
    a(
        EGFXSET,
        "BluesDriver.zip",
        486_282_388,
        "246e10012f7d515bef9ac74c9612c5b39cbf235c1386f4eadedfc0097af9dbef",
    ),
    a(
        EGFXSET,
        "RAT.zip",
        487_361_491,
        "d28b6552e2b38c9961c5c4260a2b96d23562477a19f630733099ad9c716d0b63",
    ),
    a(
        EGFXSET,
        "egfxset_metadata.csv",
        2_274,
        "ea4cd04435130a0dc3935b1551d4e08c4cfdaf9b0647bcf1c9e7f962ac632ff0",
    ),
    a(
        EGDB_BIASFX2,
        "EGDB_BIAS_FX2_random.zip",
        1_044_812_982,
        "c8d6e5c463042b408f665df892a25629c357281d5953ea370854e5a517d94304",
    ),
];

/// Where an excerpt starts in its source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    /// A fixed time in seconds.
    At(f32),
    /// The loudest window (by DI RMS, 0.5 s hop). Deterministic, and used
    /// where nobody has listened to pick a spot. `excerpt` prints the time
    /// it resolves to, so it can be pinned later.
    Loudest,
}

/// A source file inside an unpacked archive.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub archive: &'static str,
    /// Path relative to `fixtures/cache/<dataset>/<archive without .zip>/`.
    pub member: &'static str,
}

/// What a fixture contains.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Clean DI only.
    Di { di: Source },
    /// A DI and the same performance through real hardware. Written as
    /// `<id>.di.wav` and `<id>.out.wav`, with the output time-aligned to the
    /// DI by cross-correlation (and polarity-flipped if that correlates
    /// better). Not sample-identical sources: use for ballpark comparisons,
    /// not nulls.
    Paired {
        di: Source,
        out: Source,
        /// What `out` went through.
        hardware: &'static str,
    },
}

/// A short clip committed to `fixtures/committed/`.
#[derive(Clone, Copy, Debug)]
pub struct Excerpt {
    /// File stem in `fixtures/committed/`.
    pub id: &'static str,
    pub dataset: Dataset,
    pub kind: Kind,
    pub start: Start,
    pub len_s: f32,
    /// What it's good for testing.
    pub note: &'static str,
}

impl Excerpt {
    /// File names this excerpt is written as.
    pub fn files(&self) -> Vec<String> {
        match self.kind {
            Kind::Di { .. } => vec![format!("{}.wav", self.id)],
            Kind::Paired { .. } => vec![
                format!("{}.di.wav", self.id),
                format!("{}.out.wav", self.id),
            ],
        }
    }
}

const fn src(archive: &'static str, member: &'static str) -> Source {
    Source { archive, member }
}

/// Committed excerpts. All sources are 48 kHz; stereo sources are dual mono
/// and only channel 0 is kept. Levels are left as recorded (no normalizing),
/// since input level is what drives a saturating stage.
///
/// Avoided on purpose (mislabeled upstream): EGFxSet TubeScreamer
/// `Bridge-Middle/1-18..1-23`, `Neck/6-9` in every folder, RAT `Bridge/1-22`.
pub const EXCERPTS: &[Excerpt] = &[
    // EGFxSet: single notes, clean, plus the same note through drive pedals.
    Excerpt {
        id: "egfx_clean_neck_low_e",
        dataset: EGFXSET,
        kind: Kind::Di {
            di: src("Clean.zip", "Clean/Neck/6-0.wav"),
        },
        start: Start::At(0.0),
        len_s: 3.0,
        note: "Open low E (82 Hz), neck pickup: lowest fundamental, longest decay.",
    },
    Excerpt {
        id: "egfx_clean_bridge_high_e12",
        dataset: EGFXSET,
        kind: Kind::Di {
            di: src("Clean.zip", "Clean/Bridge/1-12.wav"),
        },
        start: Start::At(0.0),
        len_s: 3.0,
        note: "High E fret 12 (659 Hz), bridge pickup: bright, sharp attack.",
    },
    Excerpt {
        id: "egfx_tubescreamer_bridge_a7",
        dataset: EGFXSET,
        kind: Kind::Paired {
            di: src("Clean.zip", "Clean/Bridge/5-7.wav"),
            out: src("TubeScreamer.zip", "TubeScreamer/Bridge/5-7.wav"),
            hardware: "Ibanez Mini Tube Screamer: overdrive 1.0, tone 0.5, level 0.5",
        },
        start: Start::At(0.0),
        len_s: 3.0,
        note: "Soft-clipping pedal reference on a mid note (E3, 165 Hz).",
    },
    Excerpt {
        id: "egfx_bluesdriver_middle_g2",
        dataset: EGFXSET,
        kind: Kind::Paired {
            di: src("Clean.zip", "Clean/Middle/3-2.wav"),
            out: src("BluesDriver.zip", "BluesDriver/Middle/3-2.wav"),
            hardware: "Boss BD-2: gain 1.0, tone 0.5, level 0.5",
        },
        start: Start::At(0.0),
        len_s: 3.0,
        note: "Amp-like overdrive pedal reference (A3, 220 Hz), middle pickup.",
    },
    Excerpt {
        id: "egfx_rat_neck_low_a",
        dataset: EGFXSET,
        kind: Kind::Paired {
            di: src("Clean.zip", "Clean/Neck/6-5.wav"),
            out: src("RAT.zip", "RAT/Neck/6-5.wav"),
            hardware: "Pro Co RAT2: distortion 1.0, filter 0.5, volume 1.0",
        },
        start: Start::At(0.0),
        len_s: 3.0,
        note: "Hard-clipping pedal reference on a low note (A2, 110 Hz).",
    },
    // Guitar-TECHS: real playing, DI plus a mic on a real amp.
    Excerpt {
        id: "gtechs_palm_mute",
        dataset: GUITAR_TECHS,
        kind: Kind::Paired {
            di: src(
                "P1_techniques.zip",
                "P1_techniques/audio/directinput/directinput_PalmMute.wav",
            ),
            out: src(
                "P1_techniques.zip",
                "P1_techniques/audio/micamp/micamp_PalmMute.wav",
            ),
            hardware: "Guitar amp, close mic (Guitar-TECHS player 1 rig)",
        },
        start: Start::Loudest,
        len_s: 4.0,
        note: "Palm-muted chugs: transient-heavy low end, tests gate and drive attack.",
    },
    Excerpt {
        id: "gtechs_chords_major",
        dataset: GUITAR_TECHS,
        kind: Kind::Paired {
            di: src(
                "P1_chords.zip",
                "P1_chords/audio/directinput/directinput_Set1_maj.wav",
            ),
            out: src(
                "P1_chords.zip",
                "P1_chords/audio/micamp/micamp_Set1_maj.wav",
            ),
            hardware: "Guitar amp, close mic (Guitar-TECHS player 1 rig)",
        },
        start: Start::Loudest,
        len_s: 4.0,
        note: "Major chords: dense intermodulation under drive.",
    },
    Excerpt {
        id: "gtechs_music",
        dataset: GUITAR_TECHS,
        kind: Kind::Paired {
            di: src(
                "P3_music.zip",
                "P3_music/audio/directinput/directinput_01.wav",
            ),
            out: src("P3_music.zip", "P3_music/audio/micamp/micamp_01.wav"),
            hardware: "Guitar amp, close mic (Guitar-TECHS player 3 rig)",
        },
        start: Start::Loudest,
        len_s: 4.0,
        note: "A musical excerpt: realistic dynamics and phrasing.",
    },
    Excerpt {
        id: "gtechs_pinch_harmonics",
        dataset: GUITAR_TECHS,
        kind: Kind::Di {
            di: src(
                "P1_techniques.zip",
                "P1_techniques/audio/directinput/directinput_PinchHarmonics.wav",
            ),
        },
        start: Start::Loudest,
        len_s: 4.0,
        note: "Pinch harmonics: strong high partials, good aliasing stimulus.",
    },
    Excerpt {
        id: "gtechs_bends",
        dataset: GUITAR_TECHS,
        kind: Kind::Di {
            di: src(
                "P2_techniques.zip",
                "P2_techniques/audio/directinput/directinput_Bendings.wav",
            ),
        },
        start: Start::Loudest,
        len_s: 4.0,
        note: "String bends: sliding pitch, catches aliasing that moves against the note.",
    },
];

pub fn excerpt(id: &str) -> Option<&'static Excerpt> {
    EXCERPTS.iter().find(|e| e.id == id)
}

pub fn archive(dataset: &str, file: &str) -> Option<&'static Archive> {
    ARCHIVES
        .iter()
        .find(|a| a.dataset.id == dataset && a.file == file)
}
