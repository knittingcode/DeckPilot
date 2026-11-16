---
title: Gestures
description: Give a key up to three meanings with single click, double click and long press, tune the timings, and use the Work Session key.
group: design
order: 6
---

A key can do one thing the moment you press it, or up to three things depending on how you press it.

## Immediate keys

A key with only a single click fires as soon as it is pressed: **Single click (immediate)**. Such a key can also do something when it is released: **Release (immediate)**.

## Three gestures

In the Layout Designer, select a key and open **Gestures** in the inspector. There are three rows: **Single click**, **Double click** and **Long press**. Each has a target (an action, a workflow or a navigation step), an optional label, and **Override parameters for this gesture** to run the same action with different values.

Adding a double click or a long press changes how the key behaves, so DeckPilot asks first (**Switch Key N to gestures?**):

- A release binding is removed: press and release cannot be mixed with gestures on one key.
- With a long press, the single click fires when you release the key.
- With a double click, the single click waits for the double-click window to pass.

## Timings

| Setting | Default | Range |
| --- | --- | --- |
| Double-click window | 300 ms | 150 to 1000 ms |
| Long-press time | 700 ms | 300 to 5000 ms |

Set the defaults for this computer in **Settings › Devices**, card **Gesture timing** (**Save**, **Restore defaults**). A single key can override them in the inspector; **Use default** goes back to the computer's values.

## Dials and touch zones

The gesture editor also knows turning a dial and tapping, long-pressing or swiping a touch zone. These controls exist only on the Stream Deck +, which has no driver, so they can be designed but not used.

## The Work Session key

Work Session is a time tracker that is started and finished from one key.

1. In the Layout Designer, select a key and pick **Work Session**, then **Work Session controller**.
2. Click the key (**Start Work**) to start a session. The key shows the elapsed time.
3. To finish with an end verification workflow configured, click the key: the check runs, and when it passes the key offers **Finish Work**; click it to stop. Without a verification workflow, hold the key instead (**Hold to finish**).

The **Configure › Work Session** page shows the current session and its history; it has no start or finish buttons of its own. Its settings include a **Start workflow**, an **End verification workflow**, how long an approval lasts, and when a gap (for example the computer sleeping) is flagged in the history. Only one session can be active at a time.
