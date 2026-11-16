---
title: Wtyczki
description: Wtyczki są eksperymentalne i nie wchodzą w skład żadnej wydanej wersji DeckPilota. Co istnieje dziś, jak ma działać instalacja i gdzie to śledzić.
group: run
order: 12
experimental: true
---

> **W żadnej wydanej wersji DeckPilota nie da się instalować wtyczek.** System wtyczek jest eksperymentalny, wyłączony we wszystkich wydaniach, a w rejestrze nie ma opublikowanych wtyczek. Wszystko, co DeckPilot dziś potrafi, pochodzi z wbudowanych modułów ([Akcje](../actions/#wbudowane-moduły)).

## Jak jest dziś

- Wydany DeckPilot nie ma strony Plugins ani kodu wtyczek. Nie ma czego włączać.
- System wtyczek istnieje w kodzie źródłowym DeckPilota jako praca eksperymentalna, za przełącznikami, które są domyślnie wyłączone i których proces wydawniczy nie ustawia.
- [Plugin Marketplace](https://knittingcode.github.io/deckpilot-plugins/pl/) to osobna witryna. Pokazuje, jak Marketplace będzie działać, ale jego rejestr jest pusty: klucze podpisujące rejestr produkcyjny nie zostały jeszcze utworzone.
- Nie ma terminu, w którym wtyczki trafią do wydania.

## Jak zaprojektowano instalowanie wtyczek

Poniższy opis dotyczy eksperymentalnej implementacji z wersji rozwojowych. Zanim trafi do wydania, może się zmienić.

- **Gdzie**: strona **Plugins** w **Settings › Extensions**, z kartami **Discover**, **Installed** i **Updates**.
- **Skąd**: z podpisanego rejestru. Plik pakietu można zainstalować z dysku tylko wtedy, gdy jego zawartość odpowiada wersji wymienionej w zweryfikowanym rejestrze; instalowanie z niesprawdzonych źródeł nie istnieje.
- **Zgoda**: instalacja otwiera przegląd tego, co wtyczka może robić. Wyrażasz zgodę na każde uprawnienie z osobna, a wtyczka działa tylko wtedy, gdy ma wszystkie uprawnienia, które deklaruje. Uprawnienia można później cofnąć.
- **Osobne włączenie**: wtyczka jest instalowana jako wyłączona i włącza się ją własnym przełącznikiem.
- **Piaskownica**: wtyczki to moduły WebAssembly działające w osobnym, odizolowanym procesie. Dostępu do systemu plików, procesów i schowka nie da się przyznać.
- **Aktualizacje i usuwanie**: aktualizacje sprawdza się na tej samej stronie; aktualizacja, która prosi o więcej uprawnień, wymaga ponownej zgody. Odinstalowanie usuwa pliki wtyczki, jej uprawnienia, dane, ustawienia i poświadczenia, chyba że zdecydujesz się zachować ustawienia albo poświadczenia.

## Dla twórców wtyczek

Tworzenie wtyczek (SDK, manifest, uprawnienia, testowanie i publikowanie) opisuje witryna Marketplace, a nie ta dokumentacja: [portal dla twórców wtyczek](https://knittingcode.github.io/deckpilot-plugins/pl/developers/).
