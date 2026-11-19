// Build the portable extractor and MP2K synthesizer from their pinned sources.
pub fn build() {
    let root = "vendor/gba-audio-tools/core";
    println!("cargo:rerun-if-changed={root}");
    println!("cargo:rerun-if-changed=native/audio_bank.c");
    let mut build = cc::Build::new();
    build.include(root).std("c99").opt_level(2);
    for source in [
        "extractor/extractor.c",
        "extractor/extractor_mp2k.c",
        "extractor/extractor_webfoot.c",
        "mp2k/mp2k.c",
    ] {
        build.file(format!("{root}/{source}"));
    }
    build.file("native/audio_bank.c").compile("emerald_audio");
}
