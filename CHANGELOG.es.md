# Cambios por versión

**Español** · [English](CHANGELOG.md)

Qué trajo y qué arregló cada versión de YesCharge, de la más nueva a la más vieja. Las versiones publicadas tienen su
exe, sus notas en inglés y en español y su SHA256 en [Releases](https://github.com/Yesid-Robayo/YesCharge/releases).

Al publicar una versión nueva, se añade aquí arriba con lo mismo que su sección *Novedades* de las notas de la release.

## 0.1.0

Primera versión. Es un proyecto personal en beta: **se usa bajo tu propio riesgo** y puede que no funcione con tu
dispositivo.

### Añadir dispositivos

- **Asistente paso a paso**: eliges tu dispositivo de la lista y YesCharge prueba, en orden, la batería que Windows ya
  conoce (Bluetooth y mandos compatibles con Xbox), la batería HID estándar y lo que el dispositivo envía por su cuenta.
- **Tu propio perfil escuchando**: para los dispositivos 2.4 GHz que Windows no conoce, sigues cinco pasos (sin cable,
  conéctalo, quítalo, apágalo y enciéndelo) y la app aprende dónde vienen la carga, el encendido y el porcentaje. Los
  pasos con el dispositivo apagado siguen solos, porque no se puede pulsar nada.
- **Preguntar, solo si lo aceptas**: los modelos que solo dicen su batería si se les pregunta se pueden preguntar, tras
  un aviso de riesgo. Viene apagado, es por dispositivo y se limita a peticiones comprobadas.
- **Prueba también al preguntar**: el asistente sigue esos mismos pasos y te dice qué notó, para que sepas qué esperar
  antes de añadirlo.
- **La lista no repite** lo que ya añadiste, y un cartel indica que se está añadiendo.
- **Envíanos tu dispositivo**: si no se puede leer, la app guarda un informe sin datos personales para abrir un *issue*.

### En el día a día

- **Icono en la bandeja** con el nivel más bajo y un rayo mientras algo carga; menú con el nivel de cada dispositivo.
- **Avisos de Windows** de batería baja (se repite mientras siga baja) y de carga completa.
- **Margen de carga** tras el 100 %, para no desconectarlo antes de tiempo.
- **Autonomía aprendida**, **historial de 24 horas y 7 días** de cada dispositivo y tiempo que falta para cargar, cuando
  se puede estimar.
- **Lista ordenada a tu gusto**: los encendidos primero, o el orden fijo que elijas arrastrando cada dispositivo.
- **Botón Actualizar**: la lista muestra "Actualizando…" y vuelve con los datos nuevos, para que se note que hizo algo.
- **Español e inglés**: empieza en español y se cambia en los ajustes.
- **Buscar actualización** y **Desinstalar** en los ajustes, conservando o borrando tus datos.
- **API local opcional** en `127.0.0.1` (desactivada por defecto), con `/v1/status` y el formato compacto `/min`.

### Qué muestra en cada estado

- **Cargando sin número**: si un modelo no da su porcentaje por cable, no se muestra uno viejo como si fuera el actual.
- **Apagado** o **Dormido o apagado**: este último cuando el dispositivo no distingue entre los dos.
- **Último dato**: lo último que se supo, cuando el dispositivo no ha dicho nada o el PC lleva un rato sin usarse.

### Modelos comprobados

| Modelo | Porcentaje | Cargando | Apagado | Cuando termina de cargar |
|---|---|---|---|---|
| SteelSeries Arctis Nova 5 | Sí | Sí | Sí | Avisa al llegar al 100 % |
| MCHOSE L7 Ultra+ | Sí, sin cable | Sí | Sí | Avisa si el ratón dice que terminó |
| Mars68 (CAROTMAS) | Sí; sube unos 13 puntos cerca del cable | Sí | Dormido o apagado | Avisa al llegar a 100, tras el margen |
| GameSir Nova 2 Lite | Sí, sin cable | Sí | Sí | No se puede saber |

### Seguridad y privacidad

- **Solo lectura por defecto**: los dispositivos se abren sin permiso de escritura.
- **Preguntar tiene límites**: solo peticiones exactas incluidas en el exe, como mucho una vez por minuto, nunca con el
  PC inactivo y se detiene sola tras 3 errores seguidos.
- **Sin telemetría, sin cuentas y sin actualizaciones automáticas.**
- **No se inicia con Windows** salvo que lo actives en los ajustes.
- **No despierta tus dispositivos** mientras el PC está inactivo: entonces solo mira lo que no necesita preguntar.
- **Portable y ligera**: un exe de unos 5 MB, sin instalador ni runtime, y puedes guardar tus datos junto a él; la ventana
  es un proceso aparte que solo existe mientras está abierta.

### Limitaciones conocidas

- Los modelos que no avisan por su cuenta (Mars68 y GameSir) tardan hasta un minuto en mostrar un cambio.
- Jugar solo con un mando cuenta como PC inactivo para Windows, así que mientras tanto la app no pregunta.
- El aviso de carga terminada todavía no se ha probado con una carga completa real.
- Dos dispositivos idénticos no se distinguen: Windows les da el mismo identificador y la app solo ve uno.
- El exe no está firmado, así que Windows SmartScreen puede avisarte la primera vez.
