---
title: Wtyczki
description: DeckPilot instaluje wtyczki z podpisanego Marketplace. Gdzie je znaleźć, co mogą robić oraz jak działają aktualizacje i usuwanie.
group: run
order: 12
---

> **DeckPilot instaluje wtyczki ze swojego podpisanego Marketplace.** Instaluje się je z podpisanego rejestru wtyczek DeckPilota, działają w osobnej piaskownicy i dostają tylko te uprawnienia, na które się zgodzisz. Integracje z GitHubem, GitLabem i Jirą oraz powiadomienia to wtyczki wbudowane w DeckPilota.

## Co otrzymujesz

- **Gdzie**: przycisk **Marketplace** na pasku narzędzi, z kartami **Discover**, **Installed** i **Updates**. **Settings › Extensions › Plugins** pokazuje to samo, zaczynając od zainstalowanych wtyczek. Możesz przeszukiwać rejestr i przeczytać opis każdej wtyczki, jej wersję oraz uprawnienia, o które prosi.
- **Skąd**: z rejestru wtyczek DeckPilota, tego samego, który pokazuje [Plugin Marketplace](https://knittingcode.github.io/deckpilot-plugins/pl/). Plik pakietu można zainstalować z dysku tylko wtedy, gdy jego zawartość odpowiada wersji wymienionej w zweryfikowanym rejestrze; instalowanie z niesprawdzonych źródeł nie istnieje.
- **Pierwsze wtyczki**, autorstwa DeckPilota: Air Quality (stacje pomiaru jakości powietrza w Polsce, dane GIOŚ), Markets (kursy referencyjne euro EBC, przez Frankfurter), Pomodoro, Break & Hydration oraz Memory (gra na klawiszach).
- **Wbudowane**: integracje z GitHubem, GitLabem i Jirą oraz powiadomienia są dostarczane z DeckPilotem jako wtyczki i nie można ich zastąpić wtyczkami z rejestru. Po aktualizacji z wydania bez wtyczek każda skonfigurowana integracja przechodzi do swojej wtyczki; zapisanego tokenu albo serwera w sieci prywatnej wtyczka używa dopiero po Twojej zgodzie. Konfiguracje, których wtyczka jeszcze nie obsługuje, na przykład GitHub Enterprise, zostają przy wbudowanej integracji.
- **Przyciski na pasku narzędzi**: włączona wtyczka może dodać do czterech przycisków na pasku narzędzi, na lewo od powiadomień. Każdy otwiera widok rysowany przez samego DeckPilota: zmienne wtyczki z ich bieżącymi wartościami, jej akcje w Action Library albo jej ustawienia. Nazwa pojawia się po najechaniu na przycisk. Przyciski, które się nie mieszczą, są w menu **More plugin views**; kliknij przycisk prawym przyciskiem myszy, aby przesunąć go w lewo albo w prawo, a kolejność zostaje zapamiętana na tym komputerze. Wyłączenie albo odinstalowanie wtyczki usuwa jej przyciski. Wtyczka nie może dodać do aplikacji własnych stron, treści internetowych ani kodu; jej ikony są pokazywane wyłącznie jako obrazy.

## Jak działa instalacja

- **Zgoda**: instalacja otwiera przegląd tego, co wtyczka może robić. Wyrażasz zgodę na każde uprawnienie z osobna, a wtyczka działa tylko wtedy, gdy ma wszystkie uprawnienia, które deklaruje. Uprawnienia można później cofnąć.
- **Osobne włączenie**: wtyczka jest instalowana jako wyłączona i włącza się ją własnym przełącznikiem. Zainstalowaną wtyczkę można skonfigurować, a jej akcje przypisać do klawiszy jak każdą inną akcję.
- **Piaskownica**: wtyczki to moduły WebAssembly działające w osobnym, odizolowanym procesie, który jest częścią każdego instalatora. Wtyczka ma dostęp tylko do hostów sieciowych, które wymienia, do powiadomień, swoich klawiszy i własnych danych. Dostępu do systemu plików, procesów i schowka nie da się przyznać, a zapisanych poświadczeń wtyczka nigdy nie widzi bezpośrednio.
- **Zweryfikowany rejestr**: przy każdym odświeżeniu DeckPilot sprawdza rejestr kluczami głównymi wbudowanymi w aplikację: podpisany zestaw kluczy, indeks i znacznik czasu, a każdy pakiet po rozmiarze i SHA-256. Nieaktualny, cofnięty albo zmieniony rejestr zostaje odrzucony, a zainstalowane wtyczki działają dalej.
- **Aktualizacje i usuwanie**: aktualizacje sprawdza się na tej samej stronie; aktualizacja, która prosi o więcej uprawnień, wymaga ponownej zgody. Wtyczkę można przywrócić do wcześniejszej wersji. Odinstalowanie usuwa pliki wtyczki, jej uprawnienia, dane, ustawienia i poświadczenia, chyba że zdecydujesz się zachować ustawienia albo poświadczenia.
- **Wyłączanie wtyczek**: wtyczki są domyślnie włączone. Uruchom agenta z `--disable-plugins` albo ustaw `DECKPILOT_ENABLE_PLUGINS=0`, a wbudowane integracje będą działać jak dotąd.

## Dla twórców wtyczek

Tworzenie wtyczek (SDK, manifest, uprawnienia, testowanie i publikowanie) opisuje witryna Marketplace, a nie ta dokumentacja: [portal dla twórców wtyczek](https://knittingcode.github.io/deckpilot-plugins/pl/developers/). Rejestr publikuje własne wtyczki DeckPilota; nie jest jeszcze otwarty dla wtyczek innych wydawców.
