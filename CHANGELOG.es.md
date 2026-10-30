# Cambios por versión

**Español** · [English](CHANGELOG.md)

Qué trajo y qué arregló cada versión de YesCharge, de la más nueva a la más vieja. Las versiones publicadas tienen su
exe, sus notas en inglés y en español y su SHA256 en [Releases](https://github.com/Yesid-Robayo/YesCharge/releases).

Al publicar una versión nueva, se añade aquí arriba con lo mismo que su sección *Novedades* de las notas de la release.

## 0.1.0 (sin publicar todavía)

Primera versión.

- **Añade tu propio dispositivo** con un asistente que prueba, en orden, la batería que Windows ya conoce (Bluetooth y
  mandos compatibles con Xbox), la batería HID estándar y, si hace falta, **tu propio perfil escuchando** lo que el
  dispositivo envía mientras conectas y quitas el cable.
- **Solo lectura por defecto**: los dispositivos se abren sin permiso de escritura.
- **Preguntar, solo si lo aceptas**: a los modelos compatibles que solo dicen su batería si se les pregunta se les puede
  preguntar tras un aviso de riesgo. Viene apagado, es por dispositivo, se limita a peticiones comprobadas y se detiene
  sola tras errores repetidos. Cubre 4 modelos.
- **Icono en la bandeja** con el nivel más bajo y un rayo mientras algo carga; menú con el nivel de cada dispositivo.
- **Avisos** de batería baja (se repite mientras siga baja) y de carga completa.
- **Margen de carga** tras el 100 %, **autonomía aprendida** y **gráfica de 24 horas** de cada dispositivo.
- **No despierta tus dispositivos** mientras el PC está inactivo.
- **API local opcional** en `127.0.0.1` (desactivada por defecto), con `/v1/status` y el formato compacto `/min`.
- **Ligera**: un exe de unos 5 MB, sin runtime; la ventana es un proceso aparte que solo existe mientras está abierta.
