---
title: Przepływy
description: Łącz akcje w grafy z warunkami, gałęziami równoległymi, pętlami, ponowieniami i limitami czasu; uruchamiaj je klawiszem, harmonogramem lub zdarzeniem.
group: automate
order: 8
---

Przepływ wykonuje kilka akcji jako całość. Działa w agencie w tle, więc okno aplikacji może być zamknięte.

## Tworzenie przepływu

Otwórz **Workflows** na pasku nawigacji po lewej stronie okna.

- **New workflow** otwiera edytor grafu.
- **New step workflow** tworzy prostą, sekwencyjną listę kroków. Polecenie **Open in graph editor** zamienia ją później w graf; kroki zostają zachowane.

Pasek narzędzi zawiera **Save**, **Run workflow** (albo **Cancel run**), **Undo**, **Redo** i **Arrange nodes**. Przepływu z błędami nie da się zapisać, a przed uruchomieniem trzeba go zapisać.

## Węzły

Katalog z boku zawiera węzły, które można dodać; pole **Search nodes** go filtruje. Są w nim akcje Twojego profilu pogrupowane według modułów, a nową akcję dowolnego typu można utworzyć od razu z katalogu.

| Węzeł | Działanie |
| --- | --- |
| Start | Początek uruchomienia. Każdy przepływ ma dokładnie jeden. |
| Action | Wykonuje jedną z Twoich akcji. |
| Condition | Rozgałęzia na **Yes** albo **No**: węzeł się powiódł, wyjście ma określoną wartość, wyjście jest ustawione albo aktywny jest wskazany projekt. |
| Parallel i Join | Wykonują gałęzie równocześnie, a potem na nie czekają. |
| Loop | Wykonuje inny przepływ od 1 do 100 razy. |
| Sub-workflow | Wykonuje inny przepływ jako jeden krok. |
| Set variable | Zapamiętuje wartość dla dalszych węzłów tego uruchomienia. |
| Outputs | Określa, co przepływ zwraca. |
| Delay | Czeka, najwyżej 10 minut. |
| Stop | Kończy uruchomienie. |

Węzły Loop i Sub-workflow pojawiają się w katalogu, gdy istnieje inny przepływ.

### Gdy krok się nie powiedzie

Każdy węzeł ma ustawienia na wypadek niepowodzenia:

- **On failure**: **Stop the workflow** albo **Continue with Next**. Jeśli wyjście **On error** węzła jest połączone, niepowodzenie zawsze obsługuje ta ścieżka.
- **Retries**: do 5, dostępne tylko dla akcji, które można bezpiecznie powtórzyć. Czas **Wait between (ms)** podwaja się po każdej próbie. Przepływów podrzędnych nigdy się nie ponawia.
- **Timeout (seconds)**: domyślnie 30 sekund na akcję, najwyżej 10 minut na węzeł. Niektórych akcji nie da się przerwać.

## Przekazywanie wartości między krokami

Każdy parametr węzła przyjmuje wartość z akcji (**Action's value**), wartość dla tego węzła (**Value for this node**) albo wartość z wcześniejszych węzłów lub zmiennych (**From earlier nodes or variables**): jedno wyjście, jedną zmienną albo tekst złożony z kilku, z podglądem **Preview**.

Odwołanie zapisuje się jako `{{ steps.<węzeł>.output.<klucz> }}`. To zwykłe podstawienie; wyrażeń nie ma. Wszystko, do czego można się odwołać, opisuje strona [Zmienne](../variables/).

Przepływ może zadeklarować w swoim panelu wejścia **Inputs** i wyjścia **Outputs**. Przepływu z wymaganymi wejściami nie da się uruchomić bezpośrednio: jest przeznaczony do wywoływania z węzła Sub-workflow.

Wyjścia wrażliwe, takie jak wynik polecenia albo treść odpowiedzi, trafiają do dalszych węzłów, ale nigdy nie są zapisywane w historii.

## Uruchamianie przepływu

- Z aplikacji, przyciskiem **Run workflow**.
- Ze Stream Decka: umieść przepływ na klawiszu ([Układy](../layouts/)).
- Wyzwalaczem.

### Wyzwalacze

Gdy w edytorze nic nie jest zaznaczone, widać panel przepływu; wyzwalacz dodasz w części **Triggers** przyciskiem **Schedule** albo **Event**. Przepływ może mieć ich do 16, a każdy ma przełącznik **Paused**.

- **Harmonogramy**: co określony czas (od 1 minuty do 7 dni), codziennie albo w dni robocze, o godzinie czasu lokalnego.
- **Zdarzenia**: DeckPilot started, Profile activated, Active project changed, Stream Deck connected, Stream Deck disconnected.

Wyzwalacze działają tylko dla aktywnego profilu i tylko wtedy, gdy konfiguracja jest odblokowana. Niczego nie nadrabia się po czasie: wywołanie spóźnione o więcej niż 2 minuty jest pomijane, podobnie jak wywołanie, które przypada w trakcie poprzedniego uruchomienia.

## Obserwowanie uruchomienia

Wykonywane węzły są podświetlone w edytorze, a przy ponowieniach widać numer próby. Pasek stanu pokazuje działające przepływy i pozwala je anulować. Anulowanie zatrzymuje uruchomienie przed kolejnym węzłem; zablokowanie konfiguracji anuluje wszystkie uruchomienia.

W panelu właściwości lista **Runs** pokazuje 20 ostatnich uruchomień, a **Run details** zawiera części **Metrics**, **Failures** i **Timeline**. DeckPilot przechowuje 50 najnowszych uruchomień każdego przepływu i 500 w profilu. Historia nie trafia do eksportu.

## Ograniczenia

- Przepływ ma najwyżej 200 węzłów. Grafy nie mają cykli, przepływ nie może wywołać samego siebie, a zagnieżdżenie sięga najwyżej 4 poziomów.
- Jednocześnie działają najwyżej 4 przepływy, każdy w jednym uruchomieniu naraz, z maksymalnie 4 węzłami równoległymi w uruchomieniu.
- **May change the active project** to uprawnienie pojedynczego przepływu, wyłączone w nowych przepływach.
