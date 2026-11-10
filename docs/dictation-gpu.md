# Vulkan dictation verification

Windows x64, 2026-11-10. Stacked on PR #99 (`feat/dictation-warm-capture`).

Parakeet requests a discrete Vulkan GPU before an integrated GPU. A missing or pre-1.2 loader disables ggml Vulkan registration before speech initialization; the loader is delay-loaded and is not bundled. Whisper remains explicitly CPU-only, with the same registry policy in either load order.

GPU load/warmup or inference failures disable GPU attempts for the remainder of the process. The engine retains the complete clip and retries on CPU. Reliability rejection alone retries the whole clip on the GPU. Cancellation never starts recovery. GPU streams receive audio every 160 ms, 400 ms trailing silence, and a cooperative 1.5 s finalize deadline. CPU streaming keeps its existing one-second delivery and eight-second deadline. The existing cache and 120-second idle unload remain.

The one-second silent stream prewarm follows Hex's documented Streaming Finish flow and this task's requirement. Current Hex source uses eight seconds to compile more window shapes; Tuck deliberately uses one second here. Deadlines use the native cancellation token; they cannot forcibly interrupt a driver that ignores cancellation.

The installed read-only model at `%APPDATA%\hex\models\parakeet-unified-en-0.6b-Q8_0.gguf` is also referenced by Tuck's verified-model cache. Its 731,357,568 bytes hash to `4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795`, matching the catalog. No model was copied or changed. transcribe.cpp is pinned to Hex's `bcb639faa4c74a2300ba19e30360cb2355b1663a` revision for the reduced Vulkan shader build.

## Measurement protocol

Use the ignored `real_session_stop_to_text` harness with `TUCK_TEST_MODEL`, `TUCK_TEST_WAV`, optional `TUCK_TEST_WARM=1` and `TUCK_TEST_CPU=1` to force CPU. Run `cargo test --release --lib --manifest-path src-tauri/Cargo.toml real_session_stop_to_text -- --ignored --nocapture` after dot-sourcing `scripts/prepare-native.ps1`. Each case runs in a fresh process. Warm means a cached engine preloaded before simulated capture, not an earlier completed dictation. Cold means uncached engine, not an emptied Windows file or driver shader cache.

The fixture is generated locally with Windows System.Speech, 16 kHz mono 16-bit PCM: "This is a local dictation fixture. Release the shortcut and insert these words into the focused text field." Capture delivers 320-sample blocks in real time. Stop-to-text excludes keyboard/clipboard delivery and UIA checks. This is synthetic speech, not a real microphone or accuracy benchmark.

## Screenshot

`docs/screens/dictation-gpu-settings.png` is a browser fixture of the settings backend line, captured with `node src/tests/dictation-gpu-capture.mjs`; it does not prove native GPU execution.

## Results

One run per case, sequential fresh processes after builds finished, with the installed app left running unchanged. Stop-to-text is the harness's rounded 0.1-second output; these are not latency percentiles. All four streams were authoritative without batch fallback and returned the fixture sentence. The GPU cases reported `GPU: NVIDIA GeForce RTX 3060 Laptop GPU`; forced CPU cases reported `CPU (GPU disabled for this run)`.

| Backend | Engine state | Engine acquisition | Preload before capture | Stop to text |
| --- | --- | ---: | ---: | ---: |
| CPU | Cold | 477.0317 ms | None | 2.4 s |
| CPU | Warm | 4.1 us | 523.7923 ms | 2.4 s |
| Vulkan | Cold | 1.1020847 s | None | 0.2 s |
| Vulkan | Warm | 2.2 us | 1.0530709 s | 0.3 s |

Peak queued audio was 0.02 s in every case. Logs: ignored `artifacts/{cpu,gpu}-{cold,warm}.log`. This fixture demonstrates lower stop wait on this machine; it does not establish accuracy, long-recording performance, or a general speedup ratio.

| Release artifact | CPU parent | Vulkan change | Increase |
| --- | ---: | ---: | ---: |
| tuck.exe | 13,238,784 bytes | 21,196,800 bytes | 7,958,016 bytes |
| Unsigned NSIS installer | 4,232,509 bytes | 5,279,799 bytes | 1,047,290 bytes |

The parent source was archived from `origin/feat/dictation-warm-capture` into an ignored directory, with its own frontend built. Both used normal release profiles and the same toolchain. Installers were generated with `tauri bundle --bundles nsis --no-sign --ci` and a local config disabling updater signature artifacts; neither was installed or published. `dumpbin /imports` places `vulkan-1.dll` under delay-load imports in the new app executable.

## Verification and limits

Passed: `bun test` (472 tests), `bun run build`, `bun run test:ui`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` (297 library and 16 integration tests passed; 17 explicit probes ignored). Cargo commands used `--manifest-path src-tauri/Cargo.toml` after sourcing native setup. Device-free tests cover absent/old loader decisions, remembered GPU load failure, cancellation, retained full-clip recovery after stream errors/timeouts, and CPU/GPU/fallback snapshot labels.

Real hardware: NVIDIA and Intel adapters were enumerated; inference selected NVIDIA Vulkan0. Read-only installed Parakeet and Whisper Base models alternated successfully in both orders in separate processes: CPU-first in debug, GPU-first in release, including cancelled Parakeet runs followed by successful reuse. Four release real-session timing probes passed. Both normal release app builds and unsigned NSIS bundles passed. SDK 1.4.363.0 was installed locally; CI uses Hex's pinned 1.4.309.0 installation action plus SPIRV headers.

The optional all-target `cargo test --release --no-run` produced the release library test executable but failed integration targets with Cargo's conflicting abort/unwind library outputs. The completed release library executable (`target/release/deps/tuck_lib-6272871690bb635c.exe`) was invoked directly for the measurements and GPU-first interoperability probe. Default-profile tests and the normal release app build passed. The narrowed `--lib` command above avoids requesting the conflicting integration targets.

Not verified on real hardware: missing/old Vulkan loader startup, an actual GPU device-loss/OOM fault, CPU-only PCs, other drivers/adapters, real microphone/shortcut/typing delivery, idle VRAM release timing, installation/upgrade, MSIX installation, signed release packaging, or CI outcomes. Loader and failure paths have deterministic tests; idle unload retains the existing implementation. The settings screenshot is a fixture, not a native application capture. No installed Tuck process or model file was stopped, modified, or replaced; no release was published and no PR merged.
