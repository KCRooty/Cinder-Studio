# Changelog

Qué cambia en cada paso y qué se ha probado. Lo más reciente, arriba.

## 2026-11-19 — Fase 3: Entrega (fin del MVP)

- `cinder-core::deliver`: la línea de tiempo aplanada en tramos. La imagen sale de la pista de
  vídeo visible más alta, igual que en el visor de programa; los clips cortados con la cuchilla
  que siguen siendo continuos se leen de un tirón. El sonido es la suma de las pistas de audio
  sin silenciar.
- `cinder-media::render`: render fotograma a fotograma, sin pasar por el visor.
  - Cada fotograma del origen se escala a la salida sin deformarse; si no encaja, con bandas
    negras. Los huecos salen en negro.
  - El audio se mezcla por bloques de un fotograma, con las muestras exactas también a
    29,97 fps, y se codifica en AAC.
  - El índice va al principio del MP4 (`faststart`).
  - Codificador: primero x264/x265. Si la compilación de FFmpeg no los trae (las LGPL de
    Windows no los traen), prueba los de hardware: NVENC, Quick Sync, AMF, VideoToolbox y
    Media Foundation.
  - Al cancelar, el archivo a medias se borra.
- Presets:

  | Preset | Resolución | Códec | Calidad | Audio |
  |---|---|---|---|---|
  | YouTube 1080p | 1920×1080 | H.264 | CRF 18 | AAC 320 kb/s |
  | YouTube 4K | 3840×2160 | H.265 | CRF 20 | AAC 320 kb/s |
  | TikTok/Shorts | 1080×1920 | H.264 | CRF 20, máximo 60 fps | AAC 192 kb/s |
  | Máster | la del proyecto | H.265 | CRF 12, `slow` | AAC 320 kb/s |

- Página **Entrega**:
  - Ajustes: preset, nombre del archivo y carpeta (por defecto, junto al proyecto).
  - Cola de render:
    - Cada trabajo guarda una foto de la línea de tiempo, así que se puede seguir editando.
    - Se renderiza en un hilo aparte, un trabajo detrás de otro, con barra de progreso.
    - Cada trabajo se puede cancelar o quitar, y los acabados se limpian de golpe.
    - Nunca pisa un archivo que ya existe: añade «(2)», «(3)»…
  - Al cerrar la app con un render en marcha, se cancela.
- Las reglas del proyecto pasan de `AGENTS.md` a `DESARROLLO.md`.

Probado:
- 59 tests y clippy sin avisos. Entre ellos, render real del clip de prueba en H.264 (con un
  hueco en medio y audio) y en H.265 vertical; se comprueban la resolución, los códecs, los
  25 fps y los 3 s exactos del resultado.
- Fotogramas sacados del resultado: el hueco sale en negro, el segundo tramo empieza en el
  fotograma 0 del origen y el vertical lleva las bandas. El audio suena donde toca (−24 dB) y
  calla donde no hay clip (−91 dB).
- En la app, bajo Xvfb, con un proyecto de 4 s (dos clips con un hueco): tres trabajos
  seguidos (YouTube 1080p, TikTok y Máster). Los tres acaban con 4,01 s, en 1920×1080 H.264,
  1080×1920 H.264 y 1920×1080 HEVC, con audio AAC.

## 2026-11-15 — Fase 2, primer paso: montar en la línea de tiempo

- `cinder-core`: operaciones de montaje que devuelven un cambio entero y deshacible: cuchilla,
  insertar (parte el clip si cae dentro y empuja lo de detrás), sobrescribir (recorta, parte o
  quita lo que tapa), borrar con ripple, recortar principio y final, slip, roll y slide. Si un
  paso choca, no se aplica ninguno. Nuevos `Edit::SetClip` y `Edit::Batch`.
- Vídeo y audio de una misma toma se editan juntos (enlace implícito: mismo medio, inicio,
  entrada y duración en pistas distintas).
- Página **Medios**: importar (Ctrl+I o arrastrando), lista con la ficha de cada clip; un clic
  lo abre en el visor de origen. El primer clip de un proyecto vacío fija su formato (NTSC
  incluido: 23,976, 29,97, 59,94).
- Página **Edición**: visor de origen con entrada y salida (I / O, Alt+X las quita), visor de
  programa y línea de tiempo. F9 inserta, F10 sobrescribe y Mayús+F12 añade al final, en V1 y
  A1. Ctrl+B corta en el cabezal, Retroceso quita dejando hueco, Supr (o Mayús+Retroceso)
  borra con ripple. `,` / `.` empujan un fotograma; con Alt, slip; con Mayús, slide; con Ctrl,
  roll del corte de la derecha. ↑ / ↓ saltan de corte en corte. Ratón: arrastrar mueve (con
  imán al cabezal y a los bordes, también a otra pista), arrastrar un borde recorta,
  Ctrl+rueda acerca, Mayús+Z encaja todo. Ctrl+Z / Ctrl+Mayús+Z deshacen y rehacen.
- Guardar (Ctrl+S, Ctrl+Mayús+S) y abrir (Ctrl+O) proyectos `.cinder`.
- El visor de programa reproduce la línea de tiempo pasando de clip en clip, **todavía sin
  sonido**: la mezcla de las pistas de audio es el siguiente paso.

Probado: 36 tests (más el de la tarjeta de sonido), clippy sin avisos. En la app, bajo Xvfb, con un clip de 12 s a 25 fps:
marcar 1–4 s e insertar con F9 y añadir con Mayús+F12 pone el vídeo y el audio en V1 y A1;
Ctrl+B en 00:00:04:13 corta las dos pistas y el programa enseña el fotograma 63 del medio
(el que toca); Supr borra con ripple en las dos pistas; reproducir pasa de un clip al
siguiente; arrastrar el borde recorta 10 fotogramas, arrastrar un clip se imanta al anterior y
Ctrl+Z lo deshace. Falta: probarlo en Windows, el sonido del programa, guardar con el diálogo
real (el guardado en sí tiene tests) y los bins.

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

- Repo creado con la documentación inicial: `README.md`, `DESARROLLO.md`, `ASSETS.md`,
  `docs/PAGINAS.md`, `docs/ARQUITECTURA.md` y `docs/ROADMAP.md`.
- Sin código todavía; nada que probar.
