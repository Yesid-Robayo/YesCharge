# De dónde salen las preguntas de batería

Cada fabricante usa su propio protocolo en 2.4 GHz, así que para preguntarle la batería a un modelo hay que conocer su
petición exacta. Este documento recoge cómo se obtiene y qué fuentes hay. El catálogo de la app está en
[`core/src/ask.rs`](../core/src/ask.rs).

## Regla

**Nunca se buscan probando comandos.** Enviar bytes a ciegas a la interfaz del fabricante puede cambiar ajustes, borrar
perfiles o activar el modo de actualización de firmware. Una petición solo entra en el catálogo si se conoce por una de
estas vías y se comprobó en un dispositivo real.

## Cómo se obtienen

| Método | Cómo funciona | Coste |
|---|---|---|
| **Configurador web oficial** | Muchas marcas configuran el dispositivo desde el navegador (WebHID). El JavaScript de esa página contiene sus comandos. Así salió la del Mars68, y la de MCHOSE según [mchose-linux](https://github.com/alexfrih/mchose-linux). | Bajo, si la marca tiene uno |
| **Capturar la app oficial** | Con [USBPcap](https://desowin.org/usbpcap/) y Wireshark se graba el tráfico USB mientras la app oficial muestra la batería. En la captura se ve lo que envía y, en la respuesta, el byte que coincide con el porcentaje. | Medio. Instala un driver y requiere administrador |
| **Proyectos abiertos** | Otros ya hicieron alguno de los anteriores. Ver la tabla siguiente. | Bajo, pero hay que comprobarlo |

## Proyectos abiertos

Datos de GitHub del 30 de octubre de 2026.

| Proyecto | Licencia | Cubre | Notas |
|---|---|---|---|
| [HaloBattery](https://github.com/HeyOkay/HaloBattery) | MIT | Unas 50 entradas: ratones, auriculares, teclados y mandos de muchas marcas | App de bandeja para Windows muy parecida a YesCharge. Documenta cada protocolo en `docs/protocols.md` e indica qué se probó en hardware. Se puede reutilizar citándola. |
| [libratbag](https://github.com/libratbag/libratbag) | MIT | Ratones de varias marcas | Orientado a configurar ratones. |
| [HeadsetControl](https://github.com/Sapd/HeadsetControl) | GPL-3.0 | Auriculares | No se puede copiar su código en un proyecto MIT. |
| [Solaar](https://github.com/pwr-Solaar/Solaar) | GPL-2.0 | Logitech | Igual que el anterior. |
| [OpenRazer](https://github.com/openrazer/openrazer) | GPL-2.0 | Razer | Igual que el anterior. |

## Familias de protocolos

Una familia cubre muchos modelos con una sola lógica, pero no siempre cabe en el catálogo actual, que son pasos fijos.

| Familia | Qué se sabe | Encaja en el catálogo |
|---|---|---|
| **Logitech HID++ 2.0** | La batería tiene varias funciones (`0x1000`, `0x1001`, `0x1004`), y cuál usa cada modelo se descubre al consultarlo ([parche del kernel de Linux](https://lore.kernel.org/all/20210104182937.1472673-1-lains@archlinux.org/T/)). | No: hace falta una conversación en dos pasos |
| **Razer** | Reporte de 90 bytes, clase `0x07`, comando `0x80`, y un *transaction ID* que varía según el modelo ([referencia](https://hyperb1iss.github.io/uchroma/reference/razer-protocol-reference.html)). Los auriculares son distintos. | Todavía no: ver abajo |
| **MCHOSE `5253` / `3837`** | Misma petición que la del L7 Ultra+ en L7, M7, A7 y K7, según HaloBattery. | Sí, pero sin probar en esos modelos |
| **SteelSeries Arctis** | Varían según el modelo. | Sí, modelo a modelo |

## Modelos del catálogo

Solo están los modelos probados en un dispositivo real: SteelSeries Arctis Nova 5, MCHOSE L7 Ultra+, Mars68 (CAROTMAS) y
GameSir Nova 2 Lite.

## Cómo se añade un modelo

1. El usuario hace la prueba del asistente y nos envía el informe en un *issue* ([plantilla](../.github/ISSUE_TEMPLATE/device.yml)).
2. Con el informe vemos sus interfaces y lo que envía por su cuenta. Si eso basta, añadimos sus avisos.
3. Si solo responde cuando se le pregunta, hace falta la petición. El usuario hace una captura con la
   [guía](capturar-peticiones.md), o la buscamos en las fuentes de arriba.
4. Analizamos la captura con el porcentaje que apuntó, para ver qué petición lo trae en su respuesta:
   `cargo run -p yescharge-core --example diag -- sniff captura.pcapng 53 5253:1020`. Este análisis solo se ha probado con
   capturas simuladas.
5. Añadimos la petición al catálogo ([`core/src/ask.rs`](../core/src/ask.rs)) y publicamos una versión nueva.
6. Quien la pidió la prueba en su dispositivo. Si no funciona, se quita.
