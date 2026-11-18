---
title: Urządzenia
description: Które modele Stream Deck obsługuje DeckPilot, jak wygląda podłączone urządzenie w aplikacji i jakie ma ustawienia, na przykład jasność.
group: start
order: 3
---

## Obsługiwane modele

| Model | Stan | macOS | Windows | Linux |
| --- | --- | --- | --- | --- |
| Stream Deck MK.2 | Sterownik w zestawie | Sprawdzony na urządzeniu | Tylko testy automatyczne | Tylko testy automatyczne |
| Stream Deck XL | Tylko projektowanie układu | Brak sterownika | Brak sterownika | Brak sterownika |
| Stream Deck + | Tylko projektowanie układu | Brak sterownika | Brak sterownika | Brak sterownika |

Podłączyć i używać można wyłącznie modelu **Stream Deck MK.2** (15 klawiszy w siatce 5 × 3). Nikt jeszcze nie używał fizycznego MK.2 z DeckPilotem w systemach Windows i Linux; obejmują je testy automatyczne.

Modele XL i Stream Deck + nie mają sterownika. DeckPilot zna ich kształt, więc można zaprojektować dla nich układ, ale samo urządzenie nie zadziała. Pokrętła i strefy dotykowe ma tylko Stream Deck +, dlatego na prawdziwym sprzęcie nie da się ich dziś używać.

## Podłączanie urządzenia

Podłącz Stream Decka. DeckPilot szuka urządzeń co dwie sekundy, więc pojawi się ono po chwili samo; niczego nie trzeba parować.

Podłączone urządzenia znajdziesz w **Settings › Devices**, na karcie **Connected devices**: nazwa modelu, numer seryjny i znacznik **Connected** albo **Disconnected**. Przycisk **Edit layout** otwiera Device Editor w oknie głównym. Podłączone urządzenia pokazuje też lista urządzeń na pasku narzędzi. Urządzenia pokazuje też ekran Dashboard.

Jeśli lista jest pusta, karta zawiera komunikat **No Stream Deck detected**. Użyj przycisku **Refresh devices** na pasku tytułu albo polecenia **Refresh Devices** w menu zasobnika i zajrzyj na stronę [Rozwiązywanie problemów](../troubleshooting/#stream-deck-nie-jest-wykrywany).

W systemie Linux urządzenie wymaga reguły udev: zobacz [Instalacja](../installation/#linux).

## Jasność

Karta **Brightness** w **Settings › Devices** ma jeden suwak, **Device brightness**, w zakresie od 0 do 100% z krokiem 5, oraz przycisk **Apply**. Ustawienie dotyczy wszystkich podłączonych urządzeń; osobnej jasności dla każdego urządzenia nie ma.

## Modele, dla których można projektować

Sekcja **Supported Models** pokazuje modele sprawdzone na prawdziwym sprzęcie. Włącz **Show experimental models**, aby zobaczyć modele niesprawdzone (XL i Stream Deck +), i wybierz **Design a layout**, żeby zacząć dla nich układ. Aplikacja informuje, gdy model nie został fizycznie sprawdzony w Twoim systemie.

## Kilka urządzeń

Kod DeckPilota obsługuje więcej niż jedno urządzenie, ale praca z dwoma fizycznymi urządzeniami jednocześnie nie była testowana.
