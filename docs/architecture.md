# YesCharge architecture · Arquitectura de YesCharge

[English](#english) · [Español](#español)

## English

How YesCharge is built and why. To use it, see the [README](../README.md).

### Principle: read-only by default

YesCharge reads without sending anything, unless the user accepts otherwise for a specific device.

- [`core/src/hid.rs`](../core/src/hid.rs) only lists HID interfaces and reads input reports; `HidDevice::open` opens with
  `GENERIC_READ`.
- [`core/src/sources.rs`](../core/src/sources.rs) can make the standard *Battery Strength* `GET_REPORT`, and only if the
  device's own descriptor declares it.
- [`core/src/ask.rs`](../core/src/ask.rs) is **the only code that sends data**. Its catalog is compiled into the exe;
  `config.json` only stores the id and when the warning was accepted.

#### Why "asking" exists

Listening isn't enough: a tested mouse reports its cable and link, but its percentage doesn't appear in any message. The
only way to get it is to ask, and the request is different for each model, so it has to be known in advance. Trying
commands blindly is ruled out: that's what can change settings or leave a device unusable. So only catalog models are
asked, never an unknown one.

### Two processes, one exe

```
YesCharge.exe [--background]        YesCharge.exe --window
┌──────────────────────────┐        ┌──────────────────────────┐
│ service (tray)           │◄──────►│ window (egui)             │
│ - icon and menu          │ Post-  │ - device list             │
│ - polling thread         │ Message│ - "Add device" wizard     │
│ - Windows notifications  │        │ - details and settings    │
│ - local API (optional)   │        └────────────┬─────────────┘
└────────────┬─────────────┘                     │
             │ writes                      reads │ writes
             ▼                                   ▼
        status.json  ◄──────── %APPDATA%\YesCharge ───────► config.json
```

- **The service** uses the Windows API directly (no egui or OpenGL), so in the tray it takes a few MB. It has a hidden
  window of class `YesChargeService` to receive commands.
- **The window** is another process that only exists while it's open: closing it frees all its memory.
- **Communication**: the window writes `config.json` and sends `PostMessage` to the service (`refresh`, `reload`, `quit`,
  see [`app/src/ipc.rs`](../app/src/ipc.rs)); the service writes `status.json` atomically and the window rereads it when
  it changes.
- **Single instance**: a `Local\YesCharge.Service` mutex and a `Local\YesCharge.Window` one.

### Engine (`core/`)

| Module | What it does |
|---|---|
| `hid` | Lists HID interfaces (VID, PID, usage page, report lengths, names) and reads with a timeout. Read-only. |
| `sources` | Sources that send nothing: Windows Bluetooth battery (property `{104EA319…}`, 2), XInput and standard HID battery. |
| `probe` | Candidate list for the wizard and the tiered test (Windows → standard HID → listening). |
| `ask` | Asking mode (optional): catalog of verified requests and their limits. It's the only thing that sends data. See below. |
| `learn` | Learning mode: records what manufacturer interfaces send in five phases (no cable, cable, no cable, off, on) and looks for bytes that change with charging and stable 1-100 values. |
| `listen` | User listening profiles. A profile is a list of signals, because a device can give each piece of data in a different message and interface. There's one background listener per interface that keeps the last thing heard. |
| `monitor` | Polling cycle: median smoothing, margin after 100 %, repeated alerts, trend, battery life and the rule of not checking while the PC is idle. |
| `history` | 7-day history and battery life model learned in 10 % steps. |
| `i18n` | Interface language: each text sits where it's used, in both languages (`tr`, `trf!`). Logs aren't translated. |
| `config`, `status`, `paths`, `log` | Files in `%APPDATA%\YesCharge`, always validated and written atomically. |

#### What a catalog entry can have

| Field | What it's for | Sends anything |
|---|---|---|
| `targets`, `request`, `parse` | The exact request and how to read its answer. | Yes |
| `status` | Asks the receiver first whether the device is there. If it isn't, nothing else is sent. | Yes |
| `events` | Notices the device sends on its own. They give the cable and power state at once. | No, only listens |
| `on_cable` | The USB id it shows up with on the cable. Seeing it is enough to know it's charging. | No |
| `state_on_cable` | On the cable its percentage isn't current, but it answers whether it's charging or full. | Yes |
| `receiver_alone` | The USB id of the receiver with nothing on the other side. Seeing it means the device is off. | No |

Models with no notices of their own are asked once a minute (`Config::cycle_seconds`), the only way to notice the cable or
that they were turned off. The others follow the interval in the settings.

### Wizard flow

1. `probe::candidates()` groups HID interfaces by VID:PID, adds Bluetooth devices with a battery and XInput controllers,
   and the wizard hides what's already added (`Source::device_keys`).
2. `probe::probe()` tries Windows and standard HID, and reports each step to animate the window.
3. If nothing works and the device has manufacturer interfaces, `learn::Listener` listens through the five phases.
4. `learn::analyze()` proposes bytes; the user picks the one that matches their battery and `learn::build_profile()`
   creates a `ListenProfile`.
5. If the model can be asked and the user accepts the warning, the same five phases run, also asking in the cable, off
   and on phases, and the wizard shows what it noticed. The notices it finds are kept with the device.
6. The result is shown and the wizard asks "does it match?". It's only saved if the user confirms, and the wizard waits
   for the service to read the device before going back.

### Files

| File | Contents |
|---|---|
| `config.json` | Settings and devices, with their source: Bluetooth, XInput, standard HID, listening profile or asking. |
| `status.json` | Last status published by the service. |
| `history.json`, `model.json` | 7-day history and learned battery life. |
| `state.json` | When each device reached 100 % and the last thing known, so they survive a restart. |
| `yescharge.log` | Log with time, process and level; rotated past 512 KB. No personal data. |
| `captures\` | The last 10 listening reports. They only leave the PC if the user attaches one to an *issue*. |

A `YesChargeData` folder next to the exe makes it portable: everything is kept there. `YESCHARGE_DATA_DIR` changes the
folder for tests.

### Decisions

- **Asking is optional and separate**: everything that sends data lives in one file that's easy to audit.
- **Rust + direct Win32 in the service**: one ~5 MB exe with no runtime, a few MB of memory in the tray and no memory errors
  when reading bytes that come from devices.
- **egui only in the window**: a custom-drawn interface (rings, charts, vector icons) without loading anything graphical in
  the resident process.
- **Icons by code** ([`app/src/icon.rs`](../app/src/icon.rs)): the exe's `.ico` is generated in `build.rs`, so nothing
  binary is needed to build it.
- **System Segoe UI**: read from `C:\Windows\Fonts` when the window opens; it isn't redistributed.
- **No pull requests**: device support grows through *issues* that bring the listening report.

### Known limitations

- A device that only tells its battery when asked and isn't in the catalog can't be read.
- A listening profile only updates when the device sends its state; if it only does so when something changes, "Last
  known" is shown in between, with the time of the last message.
- Many receivers can't tell asleep from off, so the app says "Asleep or off".
- Some devices don't give a current percentage on the cable: the app shows "Charging" with no number.
- Two identical devices share a VID:PID, so candidates grouped by VID:PID only show one.

## Español

Cómo está hecho YesCharge y por qué. Para usarlo, mira el [README](../README.es.md).

### Principio: solo lectura por defecto

YesCharge lee sin enviar nada, salvo que el usuario acepte lo contrario para un dispositivo concreto.

- [`core/src/hid.rs`](../core/src/hid.rs) solo enumera interfaces HID y lee input reports; `HidDevice::open` abre con
  `GENERIC_READ`.
- [`core/src/sources.rs`](../core/src/sources.rs) puede hacer el `GET_REPORT` estándar de *Battery Strength*, y solo si el
  propio descriptor del dispositivo lo declara.
- [`core/src/ask.rs`](../core/src/ask.rs) es **el único código que envía datos**. Su catálogo está compilado en el exe;
  `config.json` solo guarda el id y el momento en que se aceptó el aviso.

#### Por qué existe "preguntar"

Escuchar no basta: un ratón probado avisa del cable y del enlace, pero su porcentaje no aparece en ningún mensaje. La
única forma de obtenerlo es preguntárselo, y la pregunta es distinta en cada modelo, así que hace falta conocerla de
antemano. Probar comandos a ciegas está descartado: es lo que puede cambiar ajustes o dejar un dispositivo inservible.
Por eso solo se pregunta a los modelos del catálogo, nunca a uno desconocido.

### Dos procesos, un exe

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

### Motor (`core/`)

| Módulo | Qué hace |
|---|---|
| `hid` | Enumerar interfaces HID (VID, PID, página de uso, largos de reporte, nombres) y leer con tiempo de espera. Solo lectura. |
| `sources` | Fuentes que no envían nada: batería Bluetooth de Windows (propiedad `{104EA319…}`, 2), XInput y batería HID estándar. |
| `probe` | Lista de candidatos para el asistente y prueba por niveles (Windows → HID estándar → escucha). |
| `ask` | Modo preguntar (opcional): catálogo de peticiones comprobadas y sus límites. Es lo único que envía datos. Ver abajo. |
| `learn` | Modo aprendizaje: graba lo que envían las interfaces del fabricante en cinco fases (sin cable, con cable, sin cable, apagado, encendido) y busca bytes que cambian con la carga y valores 1-100 estables. |
| `listen` | Perfiles de escucha del usuario. Un perfil es una lista de señales, porque un dispositivo puede dar cada dato en un mensaje e interfaz distintos. Hay una escucha de fondo por interfaz que guarda lo último que se oyó. |
| `monitor` | Ciclo de consulta: suavizado por mediana, margen tras el 100 %, avisos con repetición, tendencia, autonomía y la regla de no consultar con el PC inactivo. |
| `history` | Historial de 7 días y modelo de autonomía aprendido por tramos de 10 %. |
| `i18n` | Idioma de la interfaz: cada texto va en su sitio en los dos idiomas (`tr`, `trf!`). Los logs no se traducen. |
| `config`, `status`, `paths`, `log` | Archivos de `%APPDATA%\YesCharge`, siempre validados y escritos de forma atómica. |

#### Qué puede tener una entrada del catálogo

| Campo | Para qué sirve | Envía algo |
|---|---|---|
| `targets`, `request`, `parse` | La petición exacta y cómo leer su respuesta. | Sí |
| `status` | Pregunta antes al receptor si el dispositivo está ahí. Si no está, no se envía nada más. | Sí |
| `events` | Avisos que el dispositivo envía por su cuenta. Dan el cable y el apagado al instante. | No, solo escucha |
| `on_cable` | El identificador USB con que aparece por cable. Verlo basta para saber que carga. | No |
| `state_on_cable` | Por cable su porcentaje no es actual, pero responde si está cargando o completo. | Sí |
| `receiver_alone` | El identificador USB del receptor sin nada al otro lado. Verlo significa que está apagado. | No |

A los modelos sin avisos propios se les pregunta cada minuto (`Config::cycle_seconds`), la única forma de notar el cable
o que los apagaron. Los demás siguen el intervalo de los ajustes.

### Flujo del asistente

1. `probe::candidates()` agrupa las interfaces HID por VID:PID, añade los dispositivos Bluetooth con batería y los mandos
   XInput, y el asistente oculta lo que ya está añadido (`Source::device_keys`).
2. `probe::probe()` prueba Windows y HID estándar e informa de cada paso para animar la ventana.
3. Si no hay resultado y el dispositivo tiene interfaces del fabricante, `learn::Listener` escucha las cinco fases.
4. `learn::analyze()` propone bytes; el usuario elige cuál coincide con su batería y `learn::build_profile()` crea un
   `ListenProfile`.
5. Si el modelo se puede preguntar y el usuario acepta el aviso, se hacen las mismas cinco fases, preguntándole también
   en las de cable, apagado y encendido, y el asistente muestra qué notó. Los avisos que encuentra se guardan con el
   dispositivo.
6. Se muestra el resultado y se pregunta "¿coincide?". Solo se guarda si el usuario confirma, y el asistente espera a que
   el servicio lea el dispositivo antes de volver.

### Archivos

| Archivo | Contenido |
|---|---|
| `config.json` | Ajustes y dispositivos, con su fuente: Bluetooth, XInput, HID estándar, perfil de escucha o preguntar. |
| `status.json` | Último estado publicado por el servicio. |
| `history.json`, `model.json` | Historial de 7 días y autonomía aprendida. |
| `state.json` | Cuándo llegó cada dispositivo al 100 % y su último dato conocido, para que sobrevivan a un reinicio. |
| `yescharge.log` | Log con hora, proceso y nivel; se rota al pasar de 512 KB. Sin datos personales. |
| `captures\` | Los 10 últimos informes de escucha. Solo salen del PC si el usuario adjunta uno a un *issue*. |

Una carpeta `YesChargeData` junto al exe lo hace portable: todo se guarda ahí. `YESCHARGE_DATA_DIR` cambia la carpeta para
pruebas.

### Decisiones

- **Preguntar es opcional y aparte**: todo lo que envía datos vive en un solo archivo, fácil de auditar.
- **Rust + Win32 directo en el servicio**: un exe de unos 5 MB sin runtime, pocos MB de memoria en la bandeja y sin
  errores de memoria al leer bytes que vienen de dispositivos.
- **egui solo en la ventana**: interfaz dibujada a medida (anillos, gráficas, iconos vectoriales) sin cargar nada
  gráfico en el proceso residente.
- **Iconos por código** ([`app/src/icon.rs`](../app/src/icon.rs)): el `.ico` del exe se genera en `build.rs`, así no
  hace falta ningún binario para compilarlo.
- **Segoe UI del sistema**: se lee de `C:\Windows\Fonts` al abrir la ventana; no se redistribuye.
- **Sin pull requests**: el soporte de dispositivos crece con *issues* que traen el informe de escucha.

### Limitaciones conocidas

- Un dispositivo que solo dice su batería si se le pregunta y no está en el catálogo no se puede leer.
- Un perfil de escucha solo se actualiza cuando el dispositivo envía su estado; si lo envía solo al cambiar, entre un
  mensaje y otro se muestra "Último dato" con la hora del último mensaje.
- Muchos receptores no distinguen entre dormido y apagado, así que la app dice "Dormido o apagado".
- Algunos dispositivos no dan su porcentaje actual por cable: la app muestra "Cargando" sin número.
- Dos dispositivos idénticos comparten VID:PID, y como los candidatos se agrupan por VID:PID, solo se ve uno.
