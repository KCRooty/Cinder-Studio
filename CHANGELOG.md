# Changelog

Qué cambia en cada paso y qué se ha probado. Lo más reciente, arriba.

## 2026-11-15 — Fase 1 terminada: abrir y reproducir un clip

- Crate nuevo `cinder-media`: abre el clip con FFmpeg y lo decodifica en su propio hilo. El
  vídeo sale en RGBA para el visor y el audio en f32 estéreo, remuestreado a la frecuencia
  de la tarjeta.
- Audio con cpal. El callback solo saca muestras de una cola sin bloqueo (sin reservar
  memoria ni bloquear) y el reloj de reproducción cuenta las muestras que han sonado: el
  vídeo va detrás del audio y salta fotogramas si no llega. Sin tarjeta de sonido reproduce
  igual, con el reloj del sistema, y lo avisa.
- Saltos precisos al fotograma: se va al fotograma clave anterior y se descarta lo que sobra,
  en vídeo y en audio. Mientras se arrastra la barra solo cuenta el último salto.
- Visor en las páginas Medios y Edición: abrir con Ctrl+O, arrastrando el archivo o desde la
  línea de órdenes. Barra con reproducir/pausa, timecode HH:MM:SS:FF y barra de posición.
- Atajos: Espacio, K (pausa), L (reproducir), ← / → (un fotograma), Inicio / Fin. J, de
  momento, retrocede 1 s: la reproducción hacia atrás llega con la línea de tiempo (Fase 2).
- CI con FFmpeg en Ubuntu (apt) y en Windows (compilación LGPL compartida de BtbN, fijada).

Probado en Arch (FFmpeg 9.0.2): 21 tests (más uno que necesita tarjeta de sonido), clippy sin avisos. Con un clip de 2 s se
comprueba la ficha, el primer fotograma en pausa, saltos adelante y atrás al fotograma
exacto, la reproducción hasta el final y que un archivo que no es vídeo da error en vez de
romper. Con la tarjeta de sonido de verdad, tras 1 s el reloj marca 1 s y el vídeo va a la
par (`cargo test -p cinder-media -- --ignored`). En la app, bajo Xvfb, un clip de 12 s a
30 fps: Espacio reproduce, K para, → avanza un fotograma (el 68 enseña 00:00:02:08) y Fin
va al último (el 359, 00:00:11:29). Falta probarlo en Windows y con una tarjeta real.

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
