# Arquitectura

Documento vivo. Describe cómo se va a construir Cinder Studio; lo que cambie al
implementarlo se anota aquí y en `CHANGELOG.md`.

## Principios

1. **Un solo grafo de imagen.** El visor y el render final pasan por el mismo grafo
   (decodificación → color → composición → salida). Lo que se ve es lo que se exporta.
2. **GPU para la imagen, CPU para la lógica.** Todo píxel se procesa en wgpu; la CPU se
   encarga del proyecto, la línea de tiempo, la decodificación y el audio.
3. **Nada se congela.** Cada tarea pesada vive en su propio hilo y se comunica por canales;
   la interfaz solo pinta.
4. **Proyecto abierto.** El formato `.cinder` es texto versionado que cualquiera puede leer.

## Hilos

| Hilo | Qué hace | Puede bloquear |
|---|---|---|
| Interfaz | egui: dibuja las páginas y traduce la entrada del usuario en comandos | No |
| Proyecto | Aplica comandos al modelo (línea de tiempo, nodos), gestiona deshacer/rehacer | No |
| Decodificación (pool) | FFmpeg: lee y decodifica fotogramas por adelantado | Sí (I/O) |
| GPU | Sube fotogramas, ejecuta el grafo de imagen, presenta en el visor | No |
| Audio | Callback de cpal: mezcla en tiempo real, sin reservar memoria | Nunca |
| Render | Cola de exportación, con su propio contexto de decodificación y codificación | Sí |

El reloj maestro durante la reproducción es el de audio: el vídeo se ajusta a él y, si
no llega, salta fotogramas.

## Pipeline de imagen

```
archivo → FFmpeg (decodificación, por hardware si la hay)
        → subida a GPU (planos YUV)
        → conversión a RGB lineal en coma flotante (RGBA16F; RGBA32F en render)
        → transformación de entrada (espacio de color del clip → espacio de trabajo)
        → grafo de color
        → composición (si el clip tiene)
        → mezcla de pistas de la línea de tiempo
        → transformación de salida (espacio de trabajo → Rec.709 / sRGB / Rec.2020)
        → visor  |  codificador (Entrega)
```

- Cada nodo de color y de composición es un shader WGSL con sus parámetros; el grafo se
  compila a una lista de pasadas y solo se recalcula lo que cambia.
- Caché de fotogramas en dos niveles: fotogramas decodificados (CPU) y resultados
  intermedios de nodos caros (GPU), con límite de memoria configurable.

## Color

- Espacio de trabajo lineal por defecto; ACES (ACEScct para corregir) como opción del
  proyecto.
- LUTs `.cube` 1D y 3D, aplicadas en GPU con interpolación tetraédrica.
- Lectura de configuraciones de OpenColorIO pendiente de decidir (vía FFI o
  reimplementando lo necesario); se documentará aquí antes de empezar.
- Scopes calculados en GPU sobre la imagen ya corregida.

## Audio

- Motor de mezcla en `f32`, con la frecuencia de muestreo del proyecto.
- Grafo: clip → pista (efectos) → bus → salida principal.
- Plugins CLAP en un proceso aparte: si uno falla, se silencia esa pista y el programa
  sigue.
- Medidores de pico y loudness (EBU R128).

## Formato de proyecto `.cinder`

- Carpeta con un archivo principal en JSON (`proyecto.json`; JSON y no RON porque lo lee
  cualquier herramienta), los grafos de nodos y una subcarpeta de caché que se puede borrar
  sin perder nada.
- Campo `version` desde el primer commit; cada cambio de formato trae su migración.
- Las rutas a los medios se guardan relativas al proyecto cuando es posible, para mover
  proyectos entre equipos.

## Deshacer y rehacer

Toda modificación es un comando reversible que pasa por el hilo de proyecto. El historial
no tiene límite fijo, solo de memoria.

## Interoperabilidad

- Importar y exportar líneas de tiempo en OpenTimelineIO, FCPXML y EDL, que es lo que
  Resolve también exporta: así se puede llevar un montaje de un programa a otro.
- Plugins de vídeo OpenFX (estándar abierto, API en C) en una fase posterior.
