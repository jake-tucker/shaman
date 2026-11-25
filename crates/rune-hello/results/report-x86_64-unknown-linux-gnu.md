Built in a bare `ubuntu:24.04` with `gcc libc6-dev libxkbcommon-dev libwayland-dev` and nothing else.
### x86_64-unknown-linux-gnu
- `rustc 1.98.1 (48a229cea 2026-09-01)`, host `x86_64-unknown-linux-gnu`
- PASS `build-clean` in 99s
- debug executable: 250898336 bytes
- PASS `build-incremental` in 2s
- PASS `build-probe` in 1s
- probe's debug executable: 266117368 bytes

With no Vulkan driver installed:
- PASS `test-no-driver` in 2s
- PASS `list-scenes-no-driver` in 0s
- FAIL `snap-off-no-driver` (exit 2) in 0s
  ```
  no GPU adapter: No suitable graphics adapter found; noop support not compiled in, vulkan drivers/libraries could not be loaded, metal support not compiled in, dx12 support not compiled in, gl support not compiled in, webgpu support not compiled in
  ```
- FAIL `snap-on-no-driver` (exit 2) in 0s
  ```
  no GPU adapter: No suitable graphics adapter found; noop support not compiled in, vulkan drivers/libraries could not be loaded, metal support not compiled in, dx12 support not compiled in, gl support not compiled in, webgpu support not compiled in
  ```
- FAIL `tree-no-driver` (exit 2) in 0s
  ```
  no GPU adapter: No suitable graphics adapter found; noop support not compiled in, vulkan drivers/libraries could not be loaded, metal support not compiled in, dx12 support not compiled in, gl support not compiled in, webgpu support not compiled in
  ```
- FAIL `snap-probe-no-driver` (exit 2) in 0s
  ```
  no GPU adapter: No suitable graphics adapter found; noop support not compiled in, vulkan drivers/libraries could not be loaded, metal support not compiled in, dx12 support not compiled in, gl support not compiled in, webgpu support not compiled in
  ```

After `apt-get install mesa-vulkan-drivers` (lavapipe), with no display and nothing set in the environment:
- PASS `list-scenes-lavapipe` in 0s
- PASS `snap-off-lavapipe` in 2s
- PASS `snap-on-lavapipe` in 0s
- PASS `tree-lavapipe` in 0s
- PASS `snap-probe-lavapipe` in 1s
- ICD files it installed: `asahi_icd.json gfxstream_vk_icd.json intel_hasvk_icd.json intel_icd.json lvp_icd.json nouveau_icd.json radeon_icd.json virtio_icd.json `

What the debug executable asks the dynamic loader for (`readelf -d`):
```
(NEEDED)             Shared library: [libgcc_s.so.1]
(NEEDED)             Shared library: [libm.so.6]
(NEEDED)             Shared library: [libc.so.6]
(NEEDED)             Shared library: [ld-linux-x86-64.so.2]
```

The debug executable in a second bare `ubuntu:24.04`, with none of the build's packages:
```
$ ldd rune-hello
	linux-vdso.so.1 (0x00007f79d9d3c000)
	libgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007f79d4fd2000)
	libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 (0x00007f79d4ee9000)
	libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f79d4cd6000)
	/lib64/ld-linux-x86-64.so.2 (0x00007f79d9d3e000)
$ rune-hello --list-scenes
on
exit 0
```
- PASS `build-dist` in 407s
- dist executable: 29096576 bytes
- probe's dist executable, which holds kit and a 2.5 MB font: 33171736 bytes

What the dist executable asks the dynamic loader for:
```
(NEEDED)             Shared library: [libgcc_s.so.1]
(NEEDED)             Shared library: [libm.so.6]
(NEEDED)             Shared library: [libc.so.6]
```

Shared objects named in the dist executable's strings:
```
libX11-xcb.so libX11-xcb.so.1 libX11.so libX11.so.6 libXcursor.so libXcursor.so.1 libXi.so libXi.so.6 
libc.so.6 libfontconfig.so.1 libgcc_s.so.1 libm.so.6 libwayland-client.so libwayland-client.so.0 
libxkbcommon-x11.so libxkbcommon-x11.so.0 libxkbcommon.so libxkbcommon.so.0 
```

The dist executable in a second bare `ubuntu:24.04`, with none of the build's packages and no display:
```
$ ldd rune-hello
	linux-vdso.so.1 (0x00007f8e885bd000)
	libgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007f8e88585000)
	libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 (0x00007f8e8849c000)
	libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f8e865ed000)
	/lib64/ld-linux-x86-64.so.2 (0x00007f8e885bf000)
$ rune-hello
event loop: os error at /root/.cargo/git/checkouts/rune-c9b805e9ed8a3730/983e33b/vendor/winit/src/platform_impl/linux/mod.rs:765: neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set.
exit 1
```
