# How to capture a battery request · Cómo capturar una petición de batería

[English](#english) · [Español](#español)

## English

Some devices only tell their battery when their official app asks. YesCharge can't see that question, because a program
never sees what another one sends. To add such a model we need a USB capture of the official app.

> [!WARNING]
> A USB capture records everything on that USB port. Capture only your device's port, and **don't type passwords** while
> it runs. Look at the file before sharing it.

1. Install [Wireshark](https://www.wireshark.org/) and tick **USBPcap** in its installer. It needs administrator rights
   and a restart.
2. Close the official app, YesCharge and any other battery app (their questions would get mixed in), then plug the
   device's USB receiver in.
3. Open Wireshark and start capturing on the **USBPcap** interface that has your device.
4. Open the official app and wait until it shows the battery. Write down the percentage.
5. Stop the capture and save it as `.pcapng`.
6. Attach it to your *issue* with the percentage you wrote down.

## Español

Algunos dispositivos solo dicen su batería cuando su app oficial se la pregunta. YesCharge no puede ver esa pregunta,
porque un programa nunca ve lo que envía otro. Para añadir esos modelos hace falta una captura USB de la app oficial.

> [!WARNING]
> Una captura USB registra todo lo que pasa por ese puerto USB. Captura solo el puerto de tu dispositivo y **no escribas
> contraseñas** mientras dure. Revisa el archivo antes de compartirlo.

1. Instala [Wireshark](https://www.wireshark.org/) y marca **USBPcap** en su instalador. Necesita permisos de
   administrador y reiniciar.
2. Cierra la app oficial, YesCharge y cualquier otra app de batería (sus preguntas se mezclarían), y conecta el receptor
   USB del dispositivo.
3. Abre Wireshark y empieza a capturar en la interfaz **USBPcap** donde esté tu dispositivo.
4. Abre la app oficial y espera a que muestre la batería. Apunta el porcentaje.
5. Detén la captura y guárdala como `.pcapng`.
6. Adjúntala a tu *issue* junto con el porcentaje que apuntaste.
