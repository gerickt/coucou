// Shim for windows/src/core/sound.ts at render time: the engine only calls
// Sound.play() for its reaction sounds, which the Noctalia plugin plays itself.
// Served in place of /core/sound so the vendored engine stays byte-identical.

export const Sound = {
  play(_name: string) {},
  setEnabled(_on: boolean) {},
  setVolume(_v: number) {},
  preload() { return Promise.resolve(); },
  resume() {},
  idle() {},
};
