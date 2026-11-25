---
status: accepted
---

# Render with wgpu, winit and egui instead of a game engine

Orion is a desktop-only Rust app that draws directly to the GPU with no embedded browser. We render with `wgpu` on a `winit` window and use `egui` for panels, writing the render loop, shaders, globe, camera and picking ourselves. Learning graphics programming is a goal of the project, and the scene (one sphere, one instanced buffer of dots, a few lines, a backdrop) is small enough that an engine's scene graph and asset pipeline would mostly go unused.

## Considered Options

- **Bevy**: fastest route to a first globe, but it hides the GPU pipeline we want to learn, and brings frequent breaking releases, long compiles and a large binary.
- **iced with a custom wgpu view**: a more polished interface, but pre-1.0 with thinner documentation and friction embedding a custom renderer.
- **eframe owning the window and loop**: quicker to start than plain `winit`, but gives less control over the frame.
- **Tauri or Electron with a web globe**: ruled out because the app must not embed a browser engine.

## Consequences

- Orbital maths lives in a headless core crate with no graphics dependency, so it can be tested against published reference vectors and reused for Pass prediction or a command-line tool.
- Only Metal, Vulkan and DirectX 12 are targeted; there is no OpenGL fallback for older machines.
- The interface will look like a tool, not a consumer app, for as long as it is built on `egui`.
