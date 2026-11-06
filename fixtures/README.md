# Guitar test fixtures

Real guitar audio for testing processors, in two tiers. The manifest is code:
`crates/euterpe_fixtures/src/manifest.rs`.

| Tier | Where | Size | In git |
|---|---|---|---|
| Committed excerpts | `fixtures/committed/` | ~8 MB, 10 fixtures | yes |
| Full datasets | `fixtures/cache/` | 7 GB download, ~18 GB unpacked | no (gitignored) |

All audio is **CC BY 4.0**. Credits and the changes we made are in
[ATTRIBUTION.md](ATTRIBUTION.md). Keep them if you redistribute.

## Using them in tests

```rust
let fx = euterpe_fixtures::load("gtechs_palm_mute")?;
let out = render(&mut amp, &params, &fx.di, &RenderOptions::new(fx.sample_rate));
// fx.out: the same performance through real hardware, time-aligned to fx.di
```

`euterpe_fixtures::load_all()` returns every committed fixture. Tests that need
the full datasets should call `euterpe_fixtures::cached(..)` and skip on `None`,
so the default test run and CI never touch the network.

## Committed fixtures

All are 48 kHz mono 24-bit PCM, at their recorded level (not normalized).
Paired fixtures are `<id>.di.wav` + `<id>.out.wav`.

| id | Kind | Hardware on `.out` | What it exercises |
|---|---|---|---|
| `egfx_clean_neck_low_e` | DI | | Open low E, neck pickup: lowest fundamental, long decay |
| `egfx_clean_bridge_high_e12` | DI | | High E fret 12, bridge pickup: bright, sharp attack |
| `egfx_tubescreamer_bridge_a7` | paired | Ibanez Mini Tube Screamer, OD max | Soft clipping |
| `egfx_bluesdriver_middle_g2` | paired | Boss BD-2, gain max | Amp-like overdrive |
| `egfx_rat_neck_low_a` | paired | Pro Co RAT2, distortion max | Hard clipping |
| `gtechs_palm_mute` | paired | Real amp, close mic | Transient-heavy chugs |
| `gtechs_chords_major` | paired | Real amp, close mic | Intermodulation under drive |
| `gtechs_music` | paired | Real amp, close mic | Realistic dynamics |
| `gtechs_pinch_harmonics` | DI | | Strong high partials (aliasing stimulus) |
| `gtechs_bends` | DI | | Moving pitch (aliasing that moves against the note) |

Paired outputs are a **ballpark reference**, not a null target: they're
different hardware from anything we model, the amp ones include a mic and
room, and the alignment is by cross-correlation (lag, polarity and
correlation are printed by `excerpt`). Single-note EGFxSet pairs are aligned
on the pick attack only, within ±10 ms: over a whole sustained note the
correlation locks onto the wrong cycle. The RAT pair's lag is pinned by
hand (-3 ms), because at that gain correlation gives a different lag and
polarity for every window size. A test checks every pair's output onset is
within 5 ms of its DI.

## Regenerating

```bash
just fixtures fetch            # download, verify sha256, unzip (resumable)
just fixtures fetch egfxset    # one dataset: guitar-techs | egfxset | egdb-biasfx2
just fixtures verify           # re-hash everything in the cache
just fixtures excerpt          # re-cut fixtures/committed/ from the cache
just fixtures list             # JSON: datasets, archives, excerpts, what's present
```

`fetch` needs `curl` and `unzip`. Set `EUTERPE_FIXTURE_CACHE` to share one
cache between checkouts. Excerpts marked `Start::Loudest` pick the loudest
window of the DI; `excerpt` prints the start it chose so it can be pinned.

## Known upstream issues

- EGFxSet TubeScreamer `Bridge-Middle/1-18..1-23` are really frets 17-22.
  `Neck/6-9` plays fret 10 in every folder. RAT `Bridge/1-22` may be wrong.
  Pair by pitch, not by name, for those.
- 60 EGFxSet Clean files have a data chunk one byte too long. Our WAV reader
  ignores the stray byte; `hound` rejects them.
- Guitar-TECHS DI and amp files differ in length and are offset by a few ms
  (up to 100 ms per Zenodo), and some P1 takes are 44.1 kHz. Excerpts only use
  48 kHz takes.
- EGDB BIAS FX2 is wet only. The dry signal is EGDB's `audio_DI`, which is a
  separate download not in the manifest yet.
- Not usable at all (non-commercial licenses): IDMT-SMT-Guitar (CC BY-NC-ND)
  and GOAT (CC BY-NC, request only).
