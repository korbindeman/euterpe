# Attribution

The audio in `fixtures/committed/` is excerpted from the datasets below, used
under the [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/)
license. Please cite them as their Zenodo records ask.

**Changes made:** cut to 3 to 4 s excerpts, channel 0 kept from dual-mono
stereo files, hardware outputs time-shifted (and polarity-flipped where
noted by `just fixtures excerpt`) to align with the DI, 5 ms fades added at
the cut points, and re-encoded as 24-bit PCM. Levels are unchanged. The exact
procedure is `crates/euterpe_fixtures/src/lib.rs` (`cut`).

## Guitar-TECHS

H. Pedroza, W. Abreu, R. M. Corey, I. R. Roman. *Guitar-TECHS: An Electric
Guitar Dataset Covering Techniques, Musical Excerpts, Chords and Scales Using
a Diverse Array of Hardware.* 2025. arXiv:2501.03720.
Data: https://zenodo.org/records/14963133 (CC BY 4.0).

Used for: `gtechs_*`.

## EGFxSet

H. Pedroza, G. Meza, I. R. Roman. *EGFxSet: Electric Guitar Tones Processed
Through Real Effects of Distortion, Modulation, Delay and Reverb.* ISMIR 2022
Late-Breaking Demo. Data: https://zenodo.org/record/7044411 (CC BY 4.0).

Used for: `egfx_*`.

## EGDB rendered with BIAS FX2

In the fetch manifest only; no committed excerpts.
Data: https://zenodo.org/records/12674910 (CC BY 4.0).
