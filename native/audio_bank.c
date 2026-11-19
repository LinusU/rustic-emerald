// Own one shared, relocated instrument and sequence pool for all four players.
#include "extractor/extractor.h"
#include "mp2k/mp2k.h"
#include <stdlib.h>

typedef struct {
    uint8_t *data;
    uint32_t size;
} AudioBank;

AudioBank *rustic_audio_bank(const uint8_t *rom, uint32_t size)
{
    // gSongTable in the SHA-1-checked English cartridge.
    enum { song_table = 0x6b49f0, songs = 610 };
    Extractor *source = extractor_new(rom, size, song_table);
    if (!source) return NULL;
    int ids[songs];
    for (int i = 0; i < songs; ++i) ids[i] = i;
    AudioBank *bank = calloc(1, sizeof(*bank));
    if (bank && extractor_song_count(source) >= songs)
        bank->size = extractor_build(source, ids, songs, &bank->data);
    extractor_free(source);
    if (bank && !bank->size) {
        free(bank);
        return NULL;
    }
    return bank;
}

Mp2kPlayer *rustic_audio_start(const AudioBank *bank, uint16_t song)
{
    Mp2kConfig cfg = { .sample_rate = 32768 };
    if (song == 0 || !mp2k_pak_get(bank->data, bank->size, song, &cfg))
        return NULL;
    return mp2k_new(&cfg);
}

void rustic_audio_free(AudioBank *bank)
{
    extractor_free_buffer(bank->data);
    free(bank);
}
