# Roadmap

Cada fase termina con algo que se puede usar y probar. Las fechas se ponen al empezar
cada fase, no antes.

## Fase 0 · Diseño — *en curso*

- Este repo: reglas, páginas, arquitectura y roadmap.
- Decidir el formato del archivo principal de `.cinder` (RON o JSON).

## Fase 1 · Cimientos

- Workspace de Cargo con los crates de `README.md`.
- Ventana con egui sobre wgpu, en Windows y Linux.
- Abrir un clip, decodificarlo con FFmpeg y reproducirlo en el visor con el audio
  sincronizado.
- CI que compila y pasa los tests en Windows y Linux.

## Fase 2 · Medios y Edición

- Página **Medios**: importar, bins, metadatos, visor con entrada y salida.
- Página **Edición**: línea de tiempo multipista, J/K/L, I/O, selección, cuchilla,
  recortes ripple/roll/slip/slide, inserción y sobrescritura.
- Formato `.cinder` versionado; deshacer y rehacer.

## Fase 3 · Entrega — *fin del MVP*

- Página **Entrega**: exportar la línea de tiempo a H.264/H.265 vía FFmpeg.
- Presets básicos (YouTube, TikTok, máster) y cola de render.
- Primeros casos del oráculo: el mismo montaje exportado en Resolve y en Cinder.

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
