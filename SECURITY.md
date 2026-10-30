# Security · Seguridad

[English](#english) · [Español](#español)

## English

### How to report a security problem

If you find a security flaw (for example, a way to make YesCharge send anything other than its verified requests, or to
send them without your consent, to read keystrokes, to reach
the local API from a web page or from another PC, or to make it run something or write files outside its folder),
**do not post it in an *issue***. Use GitHub's private report:

1. The repository's **Security** tab.
2. **Report a vulnerability**.
3. Describe what happens, how to reproduce it and with which version.

Only the author will see it. You will get an answer as soon as possible and, once it is fixed, a new version will be
released.

### Supported versions

Only the latest version published in *Releases* receives fixes.

### What YesCharge protects

- **By default it sends nothing.** Devices are opened with `GENERIC_READ` only, so Windows would reject a write. A
  listening profile in `config.json` can't change that: it only says which byte to read.
- **Asking is opt-in and limited.** [`core/src/ask.rs`](core/src/ask.rs) is the only code that can send data to a device.
  It needs a risk warning accepted for that device, it only sends the exact requests built into the exe (`config.json`
  only stores an id), only to the manufacturer interface they were checked on, at most once a minute and never while the
  PC is idle, and it stops after 3 errors in a row. It never tries commands on unknown devices. When a model's state can
  be told from which USB device Windows sees, nothing is sent to find that out.
- **It doesn't read your keystrokes.** Listening only opens the manufacturer's interfaces (usage pages
  `0xFF00`–`0xFFFF`), never the keyboard or mouse ones, which Windows keeps for itself anyway.
- **Without asking, the only requests that reach the device** are the standard HID `GET_REPORT` of the *Battery Strength*
  value, and only if the device declares it. It's the same read Windows does for Bluetooth devices.
- **No administrator rights, no drivers, no services installed.** Starting with Windows is a value in
  `HKCU\...\Run` that you can turn off.
- **No telemetry, no automatic updates, no accounts.** It never connects to the internet. The links in the window
  (GitHub, opening an *issue*) only open your browser when you click them.
- **The local API is off by default.** When on, it listens on `127.0.0.1` only, accepts only `GET`, has no CORS
  headers (a web page can't read its answers), checks the `Host` header (blocks *DNS rebinding*) and limits request size,
  time and simultaneous connections.
- **What it reads from disk is checked**: `config.json` is validated and every value is brought into a sensible range;
  a broken file falls back to the defaults.
- **The log has no personal data**: no PC or Windows user name, no paths with your user name.
- **Listening reports** stay on your PC (the last 10, in `%APPDATA%\YesCharge\captures`) and only leave it if you attach
  one to an *issue*. They contain the device's name, its VID:PID and the bytes it sent on its manufacturer interfaces.
  Look at them before sharing them.
- **Releases are built by GitHub Actions** from this repository's code, with a published SHA256 and a provenance
  attestation you can verify (see the README). Dependencies are locked in `Cargo.lock` with their checksums.

### What it does not protect

- Sending anything to a device is never 100 % safe. Each request in the list was checked on a single unit of that model:
  a different firmware could behave differently. If you want no risk at all, don't turn asking on.
- The exe isn't signed with a paid certificate: check the SHA256 or the attestation before running it.
- Any program running as your user can read the files in `%APPDATA%\YesCharge` (battery levels and device names) and,
  if you turn it on, the local API.

## Español

### Cómo avisar de un problema de seguridad

Si encuentras un fallo de seguridad (por ejemplo, una forma de hacer que YesCharge envíe algo distinto de sus peticiones
comprobadas, o que las envíe sin tu aceptación, de leer pulsaciones de teclado, de llegar a la API local desde una página web o desde otro PC, o de hacer que ejecute algo
o escriba archivos fuera de su carpeta), **no lo publiques en un *issue***. Usa el aviso privado de GitHub:

1. Pestaña **Security** del repositorio.
2. **Report a vulnerability**.
3. Describe qué pasa, cómo reproducirlo y con qué versión.

Solo lo verá el autor. Se responderá lo antes posible y, una vez arreglado, se publicará una versión nueva.

### Versiones con soporte

Solo la última versión publicada en *Releases* recibe arreglos.

### Qué protege YesCharge

- **Por defecto no envía nada.** Los dispositivos se abren solo con `GENERIC_READ`, así que Windows rechazaría una
  escritura. Un perfil de escucha en `config.json` no puede cambiar eso: solo dice qué byte leer.
- **Preguntar es opcional y limitado.** [`core/src/ask.rs`](core/src/ask.rs) es el único código que puede enviar datos a
  un dispositivo. Exige aceptar un aviso de riesgo para ese dispositivo, solo envía las peticiones exactas incluidas en
  el exe (`config.json` solo guarda un identificador), solo a la interfaz del fabricante en la que se comprobaron, como
  mucho una vez por minuto y nunca con el PC inactivo, y se detiene tras 3 errores seguidos. Nunca prueba comandos con
  dispositivos desconocidos. Si el estado de un modelo se distingue por el dispositivo USB que ve Windows, no se envía nada
  para saberlo.
- **No lee tus pulsaciones.** La escucha solo abre las interfaces del fabricante (páginas de uso `0xFF00`–`0xFFFF`),
  nunca las de teclado o ratón, que además Windows reserva para sí.
- **Sin preguntar, las únicas peticiones que llegan al dispositivo** son el `GET_REPORT` HID estándar del valor *Battery
  Strength*, y solo si el dispositivo lo declara. Es la misma lectura que hace Windows con los dispositivos Bluetooth.
- **Sin permisos de administrador, sin drivers, sin servicios instalados.** Iniciar con Windows es un valor en
  `HKCU\...\Run` que puedes desactivar.
- **Sin telemetría, sin actualizaciones automáticas, sin cuentas.** Nunca se conecta a internet. Los enlaces de la
  ventana (GitHub, abrir un *issue*) solo abren tu navegador cuando haces clic.
- **La API local está desactivada por defecto.** Si la activas, escucha solo en `127.0.0.1`, acepta solo `GET`, no
  tiene cabeceras CORS (una página web no puede leer sus respuestas), comprueba la cabecera `Host` (bloquea el *DNS
  rebinding*) y limita el tamaño, el tiempo y las conexiones simultáneas.
- **Lo que lee del disco se comprueba**: `config.json` se valida y cada valor se lleva a un rango sensato; un archivo
  roto vuelve a los valores por defecto.
- **El log no tiene datos personales**: ni el nombre del PC ni el de Windows, ni rutas con tu usuario.
- **Los informes de escucha** se quedan en tu PC (los 10 últimos, en `%APPDATA%\YesCharge\captures`) y solo salen de
  él si adjuntas uno a un *issue*. Contienen el nombre del dispositivo, su VID:PID y los bytes que envió por sus
  interfaces del fabricante. Revísalos antes de compartirlos.
- **Las releases las compila GitHub Actions** con el código de este repositorio, con su SHA256 publicado y una
  atestación de origen que puedes verificar (mira el README). Las dependencias están fijadas en `Cargo.lock` con sus
  sumas de comprobación.

### Lo que no protege

- Enviarle algo a un dispositivo nunca es 100 % seguro. Cada petición de la lista se comprobó en una sola unidad de ese
  modelo: con otro firmware podría comportarse distinto. Si no quieres ningún riesgo, no actives preguntar.
- El exe no está firmado con un certificado de pago: comprueba el SHA256 o la atestación antes de ejecutarlo.
- Cualquier programa que corra con tu usuario puede leer los archivos de `%APPDATA%\YesCharge` (niveles de batería y
  nombres de dispositivos) y, si la activas, la API local.
