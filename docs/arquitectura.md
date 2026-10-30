# Arquitectura de YesCharge

Cómo está hecho YesCharge y por qué. Para usarlo, mira el [README](../README.es.md).

## Principio: solo lectura por defecto

YesCharge lee sin enviar nada, salvo que el usuario acepte lo contrario para un dispositivo concreto.

- [`core/src/hid.rs`](../core/src/hid.rs) solo enumera interfaces HID y lee input reports; `HidDevice::open` abre con
  `GENERIC_READ`.
- [`core/src/sources.rs`](../core/src/sources.rs) puede hacer el `GET_REPORT` estándar de *Battery Strength*, y solo si el
  propio descriptor del dispositivo lo declara.
- [`core/src/ask.rs`](../core/src/ask.rs) es **el único código que envía datos**. Su catálogo está compilado en el exe,
  `config.json` solo guarda el id y el momento en que se aceptó el aviso.

### Por qué existe "preguntar"

Escuchar no basta: un ratón probado avisa del cable y del enlace, pero su porcentaje no aparece en ningún mensaje. La
única forma de obtenerlo es preguntárselo, y la pregunta es distinta en cada modelo, así que hace falta conocerla de
antemano. Probar comandos a ciegas está descartado: es lo que puede cambiar ajustes o dejar un dispositivo inservible.
Por eso solo se pregunta a los modelos del catálogo, nunca a uno desconocido.

## Dos procesos, un exe

```
YesCharge.exe [--background]        YesCharge.exe --window
┌──────────────────────────┐        ┌──────────────────────────┐
│ servicio (bandeja)       │◄──────►│ ventana (egui)            │
│ - icono y menú           │ Post-  │ - lista de dispositivos   │
│ - hilo de consultas      │ Message│ - asistente "Añadir"      │
│ - avisos de Windows      │        │ - detalles y ajustes      │
│ - API local (opcional)   │        └────────────┬─────────────┘
└────────────┬─────────────┘                     │
             │ escribe                       lee │ escribe
             ▼                                   ▼
        status.json  ◄──────── %APPDATA%\YesCharge ───────► config.json
```

- **El servicio** usa la API de Windows directamente (sin egui ni OpenGL), así que en la bandeja ocupa unos pocos MB.
  Tiene una ventana oculta de clase `YesChargeService` para recibir órdenes.
- **La ventana** es otro proceso que solo existe mientras está abierta: al cerrarla se libera toda su memoria.
- **Comunicación**: la ventana escribe `config.json` y manda `PostMessage` al servicio (`refrescar`, `recargar`,
  `salir`, ver [`app/src/ipc.rs`](../app/src/ipc.rs)); el servicio escribe `status.json` de forma atómica y la ventana
  lo relee cuando cambia.
- **Instancia única**: un mutex `Local\YesCharge.Service` y otro `Local\YesCharge.Window`.

## Motor (`core/`)

| Módulo | Qué hace |
|---|---|
| `hid` | Enumerar interfaces HID (VID, PID, página de uso, largos de reporte, nombres) y leer con tiempo de espera. Solo lectura. |
| `sources` | Fuentes que no envían nada: batería Bluetooth de Windows (propiedad `{104EA319…}`, 2), XInput y batería HID estándar. |
| `probe` | Lista de candidatos para el asistente y prueba por niveles (Windows → HID estándar → escucha). |
| `ask` | Modo preguntar (opcional): catálogo de peticiones comprobadas y sus límites. Es lo único que envía datos. Cada modelo puede tener además avisos conocidos, que solo se escuchan y dan el cable y el apagado al instante. |
| `learn` | Modo aprendizaje: graba lo que envían las interfaces del fabricante en cinco fases (sin cable, con cable, sin cable, apagado, encendido) y busca bytes que cambian con la carga y valores 1-100 estables. |
| `listen` | Perfiles de escucha del usuario. Un perfil es una lista de señales (batería, carga), porque un dispositivo puede dar cada dato en un mensaje e interfaz distintos. Hay una escucha de fondo por interfaz que guarda lo último que se oyó. |
| `monitor` | Ciclo de consulta: suavizado por mediana, margen tras el 100 %, avisos con repetición, tendencia, autonomía y la regla de no consultar con el PC inactivo. |
| `history` | Historial de 24 h y modelo de autonomía aprendido por tramos de 10 % (el de BatteryInfo). |
| `config`, `status`, `paths`, `log` | Archivos de `%APPDATA%\YesCharge`, siempre validados y escritos de forma atómica. |

## Flujo del asistente

1. `probe::candidates()` agrupa las interfaces HID por VID:PID y añade los dispositivos Bluetooth con batería y los
   mandos XInput.
2. `probe::probe()` prueba Windows y HID estándar e informa de cada paso para animar la ventana.
3. Si no hay resultado y el dispositivo tiene interfaces del fabricante, `learn::Listener` escucha las cinco fases.
4. `learn::analyze()` propone bytes; el usuario elige cuál coincide con su batería y `learn::build_profile()` crea un
   `ListenProfile`.
5. Se muestra el resultado y se pregunta "¿coincide?". Solo se guarda si el usuario confirma.

## Archivos

| Archivo | Contenido |
|---|---|
| `config.json` | Ajustes y dispositivos (con su fuente: Bluetooth, XInput, HID estándar o perfil de escucha). |
| `status.json` | Último estado publicado por el servicio. |
| `history.json`, `model.json` | Historial de 24 h y autonomía aprendida. |
| `state.json` | Cuándo llegó cada dispositivo al 100 % y su último dato conocido, para que sobrevivan a un reinicio. |
| `yescharge.log` | Log con hora, proceso y nivel; se rota al pasar de 512 KB. Sin datos personales. |
| `captures\` | Los 10 últimos informes de escucha. Solo salen del PC si el usuario adjunta uno a un *issue*. |

`YESCHARGE_DATA_DIR` cambia la carpeta (pruebas o modo portable).

## Decisiones

- **Preguntar es opcional y aparte**: todo lo que envía datos vive en un solo archivo, fácil de auditar.
- **Rust + Win32 directo en el servicio**: un exe de unos 5 MB sin runtime, pocos MB de memoria en la bandeja y sin
  errores de memoria al leer bytes que vienen de dispositivos.
- **egui solo en la ventana**: interfaz dibujada a medida (anillos, gráficas, iconos vectoriales) sin cargar nada
  gráfico en el proceso residente.
- **Iconos por código** ([`app/src/icon.rs`](../app/src/icon.rs)): el `.ico` del exe se genera en `build.rs`, así el
  repo no tiene binarios.
- **Segoe UI del sistema**: se lee de `C:\Windows\Fonts` al abrir la ventana; no se redistribuye.
- **Sin pull requests**: el soporte de dispositivos crece con *issues* que traen el informe de escucha.

## Limitaciones conocidas

- Un dispositivo que solo dice su batería si se le pregunta y no está en el catálogo no se puede leer.
- Un perfil de escucha solo se actualiza cuando el dispositivo envía su estado; si lo envía solo al cambiar, entre un
  mensaje y otro se muestra "Último dato" con la hora del último mensaje. Solo se sabe que está apagado si el dispositivo avisa al apagarse y al encenderse.
- Algunos ratones cambian de VID:PID al conectar el cable (identidad "por cable"); la escucha sigue la identidad que se
  eligió en el asistente.
- La interfaz está solo en español.
