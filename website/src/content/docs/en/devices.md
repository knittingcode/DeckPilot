---
title: Devices
description: Which Stream Deck models DeckPilot supports, how a connected device shows up, and the device settings such as brightness.
group: start
order: 3
---

## Supported models

| Model | Status | macOS | Windows | Linux |
| --- | --- | --- | --- | --- |
| Stream Deck MK.2 | Driver included | Physically verified | Automated tests only | Automated tests only |
| Stream Deck XL | Layout design only | No driver | No driver | No driver |
| Stream Deck + | Layout design only | No driver | No driver | No driver |

Only the **Stream Deck MK.2** (15 keys in a 5 × 3 grid) can be connected and used. Nobody has yet used a physical MK.2 with DeckPilot on Windows or Linux; those systems are covered by automated tests.

The XL and the Stream Deck + have no driver. DeckPilot knows their shape, so you can design a layout for them, but the device itself will not work. Dials and touch zones exist only on the Stream Deck +, so they cannot be used on real hardware today.

## Connecting a device

Plug the Stream Deck in. DeckPilot looks for devices every two seconds, so it appears automatically after a moment; there is nothing to pair.

Connected devices are listed under **Settings › Devices** in the **Connected devices** card, with the model name, the serial number and a **Connected** or **Disconnected** badge. **Edit layout** opens the Layout Designer for that device. The Dashboard lists the devices too.

If nothing is listed, the card says **No Stream Deck detected**. Use **Refresh devices** in the title bar, or **Refresh Devices** in the tray menu, and see [Troubleshooting](../troubleshooting/#the-stream-deck-is-not-detected).

On Linux the device needs a udev rule: see [Installation](../installation/#linux).

## Brightness

The **Brightness** card in **Settings › Devices** has one slider, **Device brightness**, from 0 to 100 % in steps of 5, and an **Apply** button. It applies to every connected device; there is no separate brightness per device.

## Models you can design for

The **Supported Models** section lists the physically verified models. Turn on **Show experimental models** to see the models that are not verified (the XL and the Stream Deck +), and choose **Design a layout** to start a layout for one. A notice tells you when a model is not physically verified on your system.

## Several devices

DeckPilot's code handles more than one device, but two physical devices at once have not been tested.
