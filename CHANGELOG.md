# Changelog

Qué cambia en cada paso y qué se ha probado. Lo más reciente, arriba.

## 2026-11-12 — Logo nuevo y página del proyecto

- Logo nuevo (`assets/logo.png`): cámara de cine violeta sobre cielo estrellado.
- Página del proyecto en [staticforge.me/cinder](https://staticforge.me/cinder/).

## 2026-11-10 — Fase 1: cimientos

- Workspace de Cargo: `crates/cinder-core` y `apps/cinder` (binario `cinder-studio`).
- `cinder-core`: línea de tiempo con pistas de vídeo y audio (clips ordenados y sin
  solaparse), deshacer/rehacer por comandos (añadir, quitar, mover clip) y proyecto
  `.cinder` en JSON con versión de formato, guardado atómico y error claro si el proyecto es
  de una versión más nueva.
- App: ventana egui sobre wgpu con las siete páginas, barra inferior y Mayús+2 … Mayús+8
  para cambiar de página (por tecla física, para que funcione con cualquier distribución de
  teclado).
- CI en Ubuntu y Windows: formato, clippy sin avisos y tests.

Probado: 15 tests (`cargo test --workspace`), clippy sin avisos. La ventana se abrió en Linux
con Vulkan por software (lavapipe) bajo Xvfb: se ve, cambia de página con clic y con Mayús+6.
No probado aún en Windows ni con una tarjeta real.

## 2026-11-10 — Fase 0: diseño

- Repo creado con la documentación inicial: `README.md`, `AGENTS.md`, `ASSETS.md`,
  `docs/PAGINAS.md`, `docs/ARQUITECTURA.md` y `docs/ROADMAP.md`.
- Sin código todavía; nada que probar.
