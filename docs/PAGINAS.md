# Las siete páginas

Cinder Studio se organiza igual que DaVinci Resolve: siete páginas en una barra
inferior, de izquierda a derecha, todas trabajando sobre el mismo proyecto y la
misma línea de tiempo. Cambiar de página no exporta ni copia nada; es otra vista del
mismo material.

Los nombres son propios (ver `AGENTS.md` §1). La columna de la derecha dice en qué
fase de [`ROADMAP.md`](ROADMAP.md) entra cada una.

| Página | Equivale a | Qué se hace aquí | Crate principal | Fase |
|---|---|---|---|---|
| **Medios** | Media | Importar clips, organizarlos en carpetas (bins), ver metadatos, generar proxies | `cinder-media` | 2 |
| **Corte** | Cut | Montaje rápido: línea de tiempo doble (general + detalle), inserción y recorte sin cambiar de herramienta | `cinder-ui` | 7 |
| **Edición** | Edit | Edición completa: pistas de vídeo y audio, recortes ripple/roll/slip/slide, transiciones, títulos | `cinder-core`, `cinder-ui` | 2 |
| **Composición** | Fusion | Composición nodal: fusiones, máscaras, keyer, tracker, transformaciones, partículas | `cinder-comp` | 6 |
| **Color** | Color | Corrección de color por nodos: ruedas primarias, curvas, cualificadores, ventanas, LUTs, scopes | `cinder-color` | 4 |
| **Audio** | Fairlight | Mezcla multipista: buses, EQ y dinámica por pista, automatización, plugins CLAP | `cinder-audio` | 5 |
| **Entrega** | Deliver | Cola de render, presets de exportación, render por clips o de la línea de tiempo entera | `cinder-render` | 3 |

## Medios

- Navegador de archivos a la izquierda, bins del proyecto a la derecha.
- Visor con reproducción, puntos de entrada y salida, y forma de onda de audio.
- Metadatos editables (escena, toma, notas) y búsqueda.
- Generación de proxies en segundo plano para material pesado.

## Corte

- Línea de tiempo superior con todo el montaje y una inferior ampliada en el cursor.
- Inserción inteligente: el clip va al hueco más cercano al cursor.
- Pensada para montar rápido; todo lo que se hace aquí es la misma línea de tiempo de
  Edición.

## Edición

- Pistas de vídeo y audio ilimitadas, con bloqueo, silencio y solo por pista.
- Herramientas: selección (A), cuchilla (B), recorte (T); recortes ripple, roll, slip y
  slide.
- Inserción, sobrescritura, sustitución y añadir al final (F9–F12).
- Transiciones, títulos, velocidad variable y fotogramas congelados.
- Inspector de clip: transformación, recorte, composición y opacidad.

## Composición

- Grafo de nodos con visor doble (entrada y resultado).
- Nodos de fusión, transformación, máscaras (polígono, B-spline, formas), keyer de
  croma, tracker de puntos y planar, desenfoques y partículas.
- Cada clip de la línea de tiempo puede tener su composición; el resultado vuelve a la
  línea de tiempo sin renderizar aparte.

## Color

- Grafo de nodos: serie (Alt+S), paralelo (Alt+P), capa (Alt+L), con mezcla y máscara
  por nodo.
- Ruedas primarias lift/gamma/gain/offset, barras de temperatura y tinte, contraste y
  pivote.
- Curvas RGB/luma y curvas de tono contra tono, saturación y luminancia.
- Cualificadores HSL y ventanas (círculo, rectángulo, polígono, curva) con seguimiento.
- LUTs `.cube` de entrada y salida; gestión de color con espacio de trabajo lineal y
  ACES opcional.
- Scopes: forma de onda, desfile RGB, vectorscopio e histograma.
- Galería de stills para comparar y copiar correcciones entre clips.

## Audio

- Mezclador con canal por pista, buses y salida principal.
- EQ paramétrico y dinámica (puerta, compresor, limitador) en cada canal.
- Automatización por pista y por clip.
- Plugins CLAP aislados en otro proceso.
- Medidores de pico y loudness (LUFS) para entregar a la norma de cada plataforma.

## Entrega

- Presets de exportación (YouTube, TikTok, máster, intermedio) y ajustes manuales.
- Códecs vía FFmpeg: H.264, H.265, AV1, ProRes y DNxHR, con codificación por hardware
  cuando la haya.
- Cola de render: varios trabajos seguidos sin tocar nada.
- Render de la línea de tiempo entera o de cada clip por separado.
