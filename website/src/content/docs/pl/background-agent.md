---
title: Agent w tle
description: Agent obsługuje urządzenie i wykonuje akcje przy zamkniętym oknie. Jak korzystać z menu w zasobniku, uruchamiać agenta przy logowaniu i go zatrzymać.
group: run
order: 10
---

DeckPilot składa się z dwóch programów. **Agent** komunikuje się ze Stream Deckiem, rysuje klawisze, wykonuje akcje i przepływy oraz udostępnia ikonę w zasobniku. **Aplikacja konfiguracyjna** to okno, w którym projektujesz. Aplikacja uruchamia agenta, jeśli ten nie działa; agent działa dalej po zamknięciu okna.

## Menu w zasobniku

| Pozycja | Działanie |
| --- | --- |
| Open DeckPilot | Otwiera aplikację konfiguracyjną. Przy zablokowanej konfiguracji ma postać **Open DeckPilot to Unlock…** |
| Active Profile | Podmenu do zmiany profilu na urządzeniu |
| Project | Pokazuje aktywny projekt |
| Connected Devices | Lista urządzeń widzianych przez agenta |
| Refresh Devices | Wyszukuje urządzenia od razu |
| About DeckPilot | Informacje o aplikacji |
| Quit DeckPilot | Zatrzymuje agenta |

Gdy konfiguracja jest zablokowana, menu zawiera informację **Locked – actions are paused**.

W systemie Linux ikona w zasobniku wymaga obsługi AppIndicator (StatusNotifier). Bez niej ikony nie ma, ale agent działa.

## Zamykanie i kończenie pracy

- Zamknięcie okna pozostawia agenta uruchomionego. W systemie macOS tak samo działa skrót ⌘Q, który zamyka tylko interfejs aplikacji.
- Polecenie **Quit DeckPilot** w menu zasobnika zatrzymuje agenta i wygasza klawisze. W systemie macOS zamyka też aplikację; w systemach Windows i Linux aplikacja pozostaje otwarta.
- W aplikacji, w **Settings › Background Agent**, znajdują się przyciski **Stop agent** i **Start agent**.

## Uruchamianie przy logowaniu

Włącz przełącznik **Start the agent when I log in** w **Settings › Background Agent**, w części **Startup**. DeckPilot rejestruje agenta wyłącznie dla Twojego konta: jako LaunchAgent w systemie macOS, wpis autostartu XDG w systemie Linux albo klucz Run bieżącego użytkownika w systemie Windows. Nie jest instalowana żadna usługa systemowa.

To samo można zrobić z terminala:

```sh
deckpilot-agent autostart enable
deckpilot-agent autostart disable
deckpilot-agent autostart status
```

## Gdy aplikacja i agent się różnią

Aplikacja i agent uzgadniają, co obsługują obie strony, więc nowsza aplikacja współpracuje ze starszym agentem w zakresie, który obie rozumieją. Gdy do siebie nie pasują, aplikacja o tym informuje, na przykład komunikatem **The running agent does not match this app** albo informacją, że agent jest zbyt stary, i proponuje **Restart agent**. Jeśli działają przepływy, przycisk ma postać **Interrupt workflows and restart**: żaden przepływ nie kończy się po cichu.

## Dzienniki i diagnostyka

Agent zapisuje jeden plik dziennika na dzień w folderze `logs` w folderze danych DeckPilota i przechowuje siedem takich plików. Aby go przeczytać, otwórz **Settings › Logs & Diagnostics** i wybierz **Show log…**.

Karta **Diagnostics** ma przycisk **Copy**, który kopiuje raport do dołączenia do zgłoszenia błędu: wersje aplikacji i agenta, system operacyjny, foldery danych i dzienników, liczbę urządzeń oraz zgodność. Raport zawiera ścieżki z Twojego komputera, ale nie zawiera treści konfiguracji.
