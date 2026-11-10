# AGENTS.md — Cinder Studio

Reglas obligatorias para cualquier agente de IA o colaborador humano.
Cinder Studio es un editor de vídeo open source de Static Forge que replica el flujo
de trabajo de DaVinci Resolve con código propio, y lo mejora.

## 1. Clean-room (no negociable)

- Prohibido descompilar, desensamblar o depurar binarios de DaVinci Resolve o de sus
  componentes (Ghidra, IDA, x64dbg…).
- Prohibido copiar código, shaders, LUTs, PowerGrades, plantillas, títulos, efectos,
  presets, iconos, skins o cualquier asset de Blackmagic Design.
- Prohibido usar "DaVinci", "Resolve", "Fusion", "Fairlight", "Blackmagic" o
  "DaVinci Neural Engine" como nombre de nada en el producto. En docs se permite solo
  de forma descriptiva ("flujo de trabajo como el de DaVinci Resolve").
- Permitido como referencia:
  - Uso del programa, manual público, grabaciones de pantalla.
  - Observación entrada → salida (el "oráculo", ver §4).
  - Formatos abiertos de intercambio: OpenTimelineIO, FCPXML, EDL, OpenFX, LUTs `.cube`,
    configuraciones de OpenColorIO y la documentación pública de ACES.
- El formato de proyecto de Resolve (`.drp`) no está documentado: no se hace ingeniería
  inversa. Para traer proyectos de Resolve se usan los formatos que Resolve exporta
  (OTIO, FCPXML, EDL).
- Si algo solo se puede resolver mirando el binario: parar y avisar.

## 2. Qué se replica 1:1 y qué se mejora

| Replicar 1:1 (memoria muscular del usuario) | Mejorar libremente |
|---|---|
| Las siete páginas, su orden y la barra inferior para cambiar de una a otra | Todo el procesado de imagen en GPU (wgpu), con cualquier fabricante de tarjeta |
| Atajos de edición: J/K/L, I/O, A selección, B cuchilla, T recorte, F9–F12; Mayús+2 … Mayús+8 para cambiar de página | Arranque rápido, sin cuenta ni activación, funciona sin conexión |
| Recortes ripple, roll, slip y slide con la misma lógica | En Linux, H.264/H.265 y AAC funcionan igual que en Windows (vía FFmpeg) |
| Grafo de nodos de color: serie, paralelo, capa (Alt+S, Alt+P, Alt+L) | Formato de proyecto abierto y versionado, legible por otras herramientas |
| Ruedas primarias (lift/gamma/gain/offset), curvas, cualificadores, ventanas | Plugins aislados para que uno roto no tumbe el programa |
| Composición nodal: nodos, conexiones, visor doble | Funciones nuevas de Static Forge |

Equivalencia = visual y de comportamiento, no byte a byte. El nombre de cada página es
propio (ver `docs/PAGINAS.md`).

## 3. Contenido propio

Iconos, LUTs, presets de exportación, títulos, transiciones y efectos se crean desde cero
o se toman de fuentes con licencia libre. Cada asset externo se lista en `ASSETS.md` con su
origen y licencia.

## 4. Oráculo de comportamiento

Core tiene la versión gratuita de DaVinci Resolve instalada legalmente. Se usa solo como
caja negra:
1. Proyecto de prueba (clips + edición o corrección) → export en Resolve →
   `tests/oracle/<caso>/expected.*`.
2. El mismo caso en Cinder → comparar: edición por estructura (OTIO/EDL exacto), imagen por
   tolerancia (PSNR/SSIM por fotograma), audio por tolerancia/espectro.
3. Cada caso describe en texto el comportamiento esperado, sin nada de Blackmagic dentro.

## 5. Stack

- **Rust** estable (edición 2024), workspace de Cargo.
- **wgpu** (Vulkan, DirectX 12 y Metal) con shaders en WGSL para todo el procesado de imagen.
- **egui** para la interfaz (modo inmediato, como Dear ImGui en Iron3D Studio).
- **FFmpeg** (vía `ffmpeg-next`) para decodificar y codificar, con decodificación por
  hardware cuando la haya (NVDEC, VA-API, D3D11VA, VideoToolbox).
- **cpal** para la salida de audio; **CLAP** para plugins de audio; **OpenFX** para plugins
  de vídeo (más adelante).
- **rayon** para paralelismo en CPU; canales de **crossbeam** entre hilos.

Windows y Linux con paridad; CI falla si uno de los dos no compila. macOS cuando haya
quien lo pruebe.

## 6. Calidad

- La reproducción nunca bloquea la interfaz; si la máquina no llega, se saltan fotogramas,
  no se congela.
- El callback de audio nunca reserva memoria, bloquea ni hace I/O.
- Pipeline de imagen en coma flotante de principio a fin; nada se recorta a 8 bits hasta la
  salida.
- El render es determinista: el mismo proyecto da el mismo archivo.
- Formato de proyecto versionado desde el primer commit.
- Sin `unwrap()`/`panic!` en rutas de usuario: errores propagados y mostrados.
- Cada feature nueva: test + entrada en `CHANGELOG.md`.
- Cero telemetría. Sin anuncios. Sin paywalls. Sin cuenta.

## 7. Licencia

Dual **MIT OR Apache-2.0** (ver `LICENSE-MIT` y `LICENSE-APACHE`). Las dependencias deben
ser compatibles.

FFmpeg se usa en su compilación **LGPL** y enlazado dinámicamente, para que el proyecto
pueda seguir siendo MIT/Apache. No se distribuyen componentes GPL de FFmpeg (p. ej. x264 o
x265) dentro de Cinder; quien los quiera puede apuntar Cinder a su propia compilación de
FFmpeg.
