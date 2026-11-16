---
title: Rozwiązywanie problemów
description: Co sprawdzić, gdy Stream Deck nie jest wykrywany, brakuje ikony w zasobniku, konfiguracja się nie otwiera albo aktualizacja się nie udaje.
group: help
order: 13
---

## Stream Deck nie jest wykrywany

W **Settings › Devices** widnieje komunikat **No Stream Deck detected**.

- Sprawdź model. Sterownik ma tylko Stream Deck MK.2 ([Urządzenia](../devices/)).
- Wybierz **Refresh devices** na pasku tytułu albo **Refresh Devices** w menu zasobnika. DeckPilot sam szuka urządzeń co dwie sekundy.
- Upewnij się, że agent działa: **Settings › Background Agent**, **Start agent**.
- Zamknij inne programy sterujące Stream Deckiem, na przykład aplikację producenta, żeby nie rywalizowały z DeckPilotem o urządzenie.
- **Linux**: musi być zainstalowana reguła udev, a urządzenie trzeba potem podłączyć ponownie. Pakiet `.deb` instaluje ją sam; przy AppImage robisz to samodzielnie ([Instalacja](../installation/#linux)).
- **macOS**: jeśli system poprosi o uprawnienie „Monitorowanie wprowadzania”, przyznaj je agentowi.
- **Windows**: sterownik nie jest potrzebny; urządzenie korzysta z wbudowanego sterownika HID.

Z prawdziwym urządzeniem DeckPilot był używany tylko w systemie macOS. Jeśli w systemie Windows albo Linux coś nie działa, [zgłoś problem](https://github.com/knittingcode/deckpilot/issues/new/choose) i dołącz raport diagnostyczny (niżej).

## Windows ostrzega przed instalatorem

Instalatory dla systemu Windows nie mają jeszcze podpisu kodu, więc filtr SmartScreen ostrzega przy pierwszym uruchomieniu. Zanim przejdziesz dalej, porównaj sumę SHA-256 pliku z podaną na [stronie pobierania](../../download/#verify-title).

## W systemie Linux nie ma ikony w zasobniku

Ikona wymaga obsługi AppIndicator (StatusNotifier), której część środowisk, na przykład GNOME bez rozszerzenia, nie ma. Agent mimo to działa i klawisze reagują; aplikację otworzysz z menu programów.

## DeckPilot nie może otworzyć konfiguracji

Przy kluczu systemowym klucz konfiguracji znajduje się w magazynie poświadczeń (pęk kluczy, Menedżer poświadczeń albo Secret Service).

- Jeśli magazyn jest zablokowany, odblokuj go albo zaloguj się ponownie i spróbuj jeszcze raz. W systemie Linux menedżer kluczy może wyświetlić własne okno odblokowania.
- Jeśli system zapyta, czy DeckPilot może korzystać z magazynu poświadczeń, zezwól na to.
- Jeśli klucz został usunięty z magazynu poświadczeń, konfiguracji nie da się odzyskać. Rozpocznij nową konfigurację i przywróć zaszyfrowany eksport poleceniem **Settings › Backup & Restore › Import…**.

Przy haśle zapomnianego hasła również nie da się odzyskać. Jedyną kopią zapasową jest zaszyfrowany eksport.

## Klawisze pokazują napis „Locked”

DeckPilot jest zablokowany i nie wykonuje akcji. Otwórz aplikację i odblokuj ją: **Unlock…** albo **Unlock with Touch ID** w systemie macOS. To, kiedy się blokuje, zmienisz w **Settings › Security**.

## Agent nie działa albo nie pasuje do aplikacji

- **Settings › Background Agent** pokazuje stan agenta i zawiera przycisk **Start agent**.
- Jeśli aplikacja informuje, że działający agent do niej nie pasuje albo jest zbyt stary, wybierz **Restart agent**. Zwykle zdarza się to po aktualizacji.
- Dziennik agenta przeczytasz w **Settings › Logs & Diagnostics**, przyciskiem **Show log…**.

## Akcja nic nie robi albo jest odrzucana

- **Run command**, **Compose up**, **Compose down** i **Compose restart** oraz żądania do sieci prywatnych są wyłączone, dopóki nie zezwolisz na nie na tym komputerze ([Akcje](../actions/#co-jest-wyłączone-dopóki-na-to-nie-zezwolisz)).
- Akcje wyłączonego modułu są oznaczone jako **Unavailable**. Włącz moduł w **Settings › Extensions › Modules**.
- Zasób w stanie **Missing** albo **Not set on this machine** wymaga przypisania w **Configure › Projects**, na karcie **This machine**.
- Przycisk **Run** w bibliotece akcji działa tylko wtedy, gdy agent jest uruchomiony, a profil jest profilem aktywnym.

## Wyzwalacz nie zadziałał

Wyzwalacze działają tylko dla aktywnego profilu i tylko przy odblokowanej konfiguracji. Wywołanie spóźnione o więcej niż dwie minuty jest pomijane, podobnie jak wywołanie, które przypada w trakcie poprzedniego uruchomienia ([Przepływy](../workflows/#wyzwalacze)).

## Aktualizacja się nie udaje

DeckPilot odrzuca aktualizację, której podpisu nie da się zweryfikować. Pobierz instalator ze [strony pobierania](../../download/) i zainstaluj go ręcznie. Dziennik aktualizacji to plik `updates.log` w folderze `logs`.

## Zgłaszanie problemu

1. Otwórz **Settings › Logs & Diagnostics** i użyj przycisku **Copy** na karcie **Diagnostics**. Raport zawiera wersje, system operacyjny i ścieżki z Twojego komputera, ale nie zawiera treści konfiguracji.
2. [Utwórz zgłoszenie](https://github.com/knittingcode/deckpilot/issues/new/choose) i wklej raport.

Podatności bezpieczeństwa zgłaszaj prywatnie, zgodnie z [polityką bezpieczeństwa](https://github.com/knittingcode/deckpilot/blob/main/SECURITY.md), a nie w publicznym zgłoszeniu.
