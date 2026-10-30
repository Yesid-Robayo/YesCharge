# Where the battery requests come from · De dónde salen las preguntas de batería

[English](#english) · [Español](#español)

## English

Each manufacturer uses its own 2.4 GHz protocol, so asking a model for its battery takes knowing its exact request. This
document covers how a request is obtained and which sources exist. The app's catalog is in
[`core/src/ask.rs`](../core/src/ask.rs).

### Rule

**Requests are never found by trying commands.** Sending bytes blindly to a manufacturer interface can change settings,
erase profiles or trigger firmware update mode. A request only enters the catalog if it's known through one of the ways
below and was checked on a real device.

### How they are obtained

| Method | How it works | Cost |
|---|---|---|
| **Official web configurator** | Many brands configure the device from the browser (WebHID). That page's JavaScript contains its commands. This is where the Mars68 request came from, and the MCHOSE one according to [mchose-linux](https://github.com/alexfrih/mchose-linux). | Low, if the brand has one |
| **Capturing the official app** | [USBPcap](https://desowin.org/usbpcap/) and Wireshark record the USB traffic while the official app shows the battery. The capture shows what it sends and, in the reply, the byte that matches the percentage. | Medium. Installs a driver and needs administrator rights |
| **Open projects** | Others have already done one of the above. See the next table. | Low, but it must be checked |

### Open projects

GitHub data from October 30, 2026.

| Project | License | Covers | Notes |
|---|---|---|---|
| [HaloBattery](https://github.com/HeyOkay/HaloBattery) | MIT | About 50 entries: mice, headsets, keyboards and controllers from many brands | Windows tray app very similar to YesCharge. It documents each protocol in `docs/protocols.md` and says what was tried on hardware. It can be reused with credit. |
| [libratbag](https://github.com/libratbag/libratbag) | MIT | Mice from several brands | Focused on configuring mice. |
| [HeadsetControl](https://github.com/Sapd/HeadsetControl) | GPL-3.0 | Headsets | Its code can't be copied into an MIT project. |
| [Solaar](https://github.com/pwr-Solaar/Solaar) | GPL-2.0 | Logitech | Same as above. |
| [OpenRazer](https://github.com/openrazer/openrazer) | GPL-2.0 | Razer | Same as above. |

### Protocol families

A family covers many models with one logic, but it doesn't always fit the current catalog, which is made of fixed steps.

| Family | What is known | Fits the catalog |
|---|---|---|
| **Logitech HID++ 2.0** | The battery has several features (`0x1000`, `0x1001`, `0x1004`), and which one a model uses is found by asking it ([Linux kernel patch](https://lore.kernel.org/all/20210104182937.1472673-1-lains@archlinux.org/T/)). | No: it needs a two-step conversation |
| **Razer** | 90-byte report, class `0x07`, command `0x80`, and a *transaction ID* that varies by model ([reference](https://hyperb1iss.github.io/uchroma/reference/razer-protocol-reference.html)). Headsets are different. | Not yet: its keyboards answer outside the manufacturer interface, the only place YesCharge sends to |
| **MCHOSE `5253` / `3837`** | Same request as the L7 Ultra+ on the L7, M7, A7 and K7, according to HaloBattery. | Yes, but untested on those models |
| **SteelSeries Arctis** | It varies by model. | Yes, model by model |

### Models in the catalog

Only models tried on a real device: SteelSeries Arctis Nova 5, MCHOSE L7 Ultra+, Mars68 (CAROTMAS) and GameSir Nova 2 Lite.

### How a model is added

1. The user runs the wizard's test and sends us the report in an *issue* ([template](../.github/ISSUE_TEMPLATE/device.yml)).
2. The report shows its interfaces and what it sends on its own. If that's enough, its notices are added.
3. If it only answers when asked, the request is needed. The user makes a capture with the [guide](capturing-requests.md),
   or it's looked up in the sources above.
4. The capture is analyzed with the percentage the user wrote down, to see which request carries it in its reply:
   `cargo run -p yescharge-core --example diag -- sniff capture.pcapng 53 5253:1020`. This analysis has only been tried on
   simulated captures.
5. The request is added to the catalog ([`core/src/ask.rs`](../core/src/ask.rs)) and a new version is published.
6. Whoever asked for it tries it on their device. If it doesn't work, it's removed.

## Español

Cada fabricante usa su propio protocolo en 2.4 GHz, así que para preguntarle la batería a un modelo hay que conocer su
petición exacta. Este documento recoge cómo se obtiene y qué fuentes hay. El catálogo de la app está en
[`core/src/ask.rs`](../core/src/ask.rs).

### Regla

**Nunca se buscan probando comandos.** Enviar bytes a ciegas a la interfaz del fabricante puede cambiar ajustes, borrar
perfiles o activar el modo de actualización de firmware. Una petición solo entra en el catálogo si se conoce por una de
estas vías y se comprobó en un dispositivo real.

### Cómo se obtienen

| Método | Cómo funciona | Coste |
|---|---|---|
| **Configurador web oficial** | Muchas marcas configuran el dispositivo desde el navegador (WebHID). El JavaScript de esa página contiene sus comandos. Así salió la del Mars68, y la de MCHOSE según [mchose-linux](https://github.com/alexfrih/mchose-linux). | Bajo, si la marca tiene uno |
| **Capturar la app oficial** | Con [USBPcap](https://desowin.org/usbpcap/) y Wireshark se graba el tráfico USB mientras la app oficial muestra la batería. En la captura se ve lo que envía y, en la respuesta, el byte que coincide con el porcentaje. | Medio. Instala un driver y requiere administrador |
| **Proyectos abiertos** | Otros ya hicieron alguno de los anteriores. Ver la tabla siguiente. | Bajo, pero hay que comprobarlo |

### Proyectos abiertos

Datos de GitHub del 30 de octubre de 2026.

| Proyecto | Licencia | Cubre | Notas |
|---|---|---|---|
| [HaloBattery](https://github.com/HeyOkay/HaloBattery) | MIT | Unas 50 entradas: ratones, auriculares, teclados y mandos de muchas marcas | App de bandeja para Windows muy parecida a YesCharge. Documenta cada protocolo en `docs/protocols.md` e indica qué se probó en hardware. Se puede reutilizar citándola. |
| [libratbag](https://github.com/libratbag/libratbag) | MIT | Ratones de varias marcas | Orientado a configurar ratones. |
| [HeadsetControl](https://github.com/Sapd/HeadsetControl) | GPL-3.0 | Auriculares | No se puede copiar su código en un proyecto MIT. |
| [Solaar](https://github.com/pwr-Solaar/Solaar) | GPL-2.0 | Logitech | Igual que el anterior. |
| [OpenRazer](https://github.com/openrazer/openrazer) | GPL-2.0 | Razer | Igual que el anterior. |

### Familias de protocolos

Una familia cubre muchos modelos con una sola lógica, pero no siempre cabe en el catálogo actual, que son pasos fijos.

| Familia | Qué se sabe | Encaja en el catálogo |
|---|---|---|
| **Logitech HID++ 2.0** | La batería tiene varias funciones (`0x1000`, `0x1001`, `0x1004`), y cuál usa cada modelo se descubre al consultarlo ([parche del kernel de Linux](https://lore.kernel.org/all/20210104182937.1472673-1-lains@archlinux.org/T/)). | No: hace falta una conversación en dos pasos |
| **Razer** | Reporte de 90 bytes, clase `0x07`, comando `0x80`, y un *transaction ID* que varía según el modelo ([referencia](https://hyperb1iss.github.io/uchroma/reference/razer-protocol-reference.html)). Los auriculares son distintos. | Todavía no: sus teclados responden fuera de la interfaz del fabricante, el único sitio al que YesCharge envía |
| **MCHOSE `5253` / `3837`** | Misma petición que la del L7 Ultra+ en L7, M7, A7 y K7, según HaloBattery. | Sí, pero sin probar en esos modelos |
| **SteelSeries Arctis** | Varían según el modelo. | Sí, modelo a modelo |

### Modelos del catálogo

Solo están los modelos probados en un dispositivo real: SteelSeries Arctis Nova 5, MCHOSE L7 Ultra+, Mars68 (CAROTMAS) y
GameSir Nova 2 Lite.

### Cómo se añade un modelo

1. El usuario hace la prueba del asistente y nos envía el informe en un *issue* ([plantilla](../.github/ISSUE_TEMPLATE/device.yml)).
2. Con el informe vemos sus interfaces y lo que envía por su cuenta. Si eso basta, añadimos sus avisos.
3. Si solo responde cuando se le pregunta, hace falta la petición. El usuario hace una captura con la
   [guía](capturing-requests.md), o la buscamos en las fuentes de arriba.
4. Analizamos la captura con el porcentaje que apuntó, para ver qué petición lo trae en su respuesta:
   `cargo run -p yescharge-core --example diag -- sniff capture.pcapng 53 5253:1020`. Este análisis solo se ha probado con
   capturas simuladas.
5. Añadimos la petición al catálogo ([`core/src/ask.rs`](../core/src/ask.rs)) y publicamos una versión nueva.
6. Quien la pidió la prueba en su dispositivo. Si no funciona, se quita.
