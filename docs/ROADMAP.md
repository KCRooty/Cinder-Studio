# Roadmap

Cada fase termina con algo que se puede usar y probar. Las fechas se ponen al empezar
cada fase, no antes.

## Fase 0 · Diseño — *hecha*

- Este repo: reglas, páginas, arquitectura y roadmap.
- Formato del archivo principal de `.cinder`: JSON (decidido el 2026-11-10).

## Fase 1 · Cimientos — *hecha*

- ✔ Workspace de Cargo (`cinder-core` y la app; el resto de crates, cuando toque).
- ✔ Ventana con egui sobre wgpu, con las siete páginas y Mayús+2 … Mayús+8.
- ✔ Abrir un clip, decodificarlo con FFmpeg y reproducirlo en el visor con el audio
  sincronizado.
- ✔ CI que compila y pasa los tests en Windows y Linux.
- ✔ Modelo de proyecto, deshacer/rehacer y guardado en `.cinder` (adelantado de la Fase 2).

## Fase 2 · Medios y Edición — *hecha*

- Página **Medios**: ✔ importar, ✔ metadatos, ✔ visor con entrada y salida, ✔ carpetas.
- Página **Edición**: ✔ línea de tiempo multipista, ✔ J/K/L, ✔ I/O, ✔ selección, ✔ cuchilla,
  ✔ recortes ripple/roll/slip/slide, ✔ inserción y sobrescritura, ✔ sonido del programa
  (mezcla de las pistas de audio), ✔ enlace de vídeo y audio guardado, ✔ añadir y quitar pistas.
- ✔ Formato `.cinder` versionado; ✔ deshacer y rehacer.

## Fase 3 · Entrega — *hecha, fin del MVP*

- ✔ Página **Entrega**: exportar la línea de tiempo a H.264/H.265 con audio AAC vía FFmpeg.
- ✔ Presets: YouTube 1080p, YouTube 4K, TikTok/Shorts (vertical) y Máster.
- ✔ Cola de render en segundo plano, con progreso, cancelar y varios trabajos seguidos.
- Pendiente: los primeros casos del oráculo (el mismo montaje exportado en Resolve y en
  Cinder), rango de entrada/salida en vez de la línea de tiempo entera, y respetar la rotación
  de los vídeos grabados con el móvil.

## Fase 4 · Color

- Grafo de nodos (serie, paralelo, capa), ruedas primarias, curvas y LUTs `.cube`.
- Cualificadores HSL y ventanas.
- Scopes: forma de onda, desfile RGB, vectorscopio e histograma.
- Gestión de color: espacio de trabajo lineal y ACES opcional.

## Fase 5 · Audio

- Mezclador con canales por pista, buses y salida principal.
- EQ y dinámica por canal, automatización, medidores de loudness.
- Plugins CLAP aislados.

## Fase 6 · Composición

- Grafo de nodos con visor doble.
- Fusión, transformación, máscaras, keyer de croma, tracker.

## Fase 7 · Corte y rendimiento

- Página **Corte**: línea de tiempo doble y montaje rápido.
- Proxies y caché optimizada para material 4K y superior.

## Fase 8 · Interoperabilidad

- Importar y exportar OpenTimelineIO, FCPXML y EDL.
- Plugins de vídeo OpenFX.
