---
title: Aktualizacje
description: Jak DeckPilot sprawdza nowe wersje w GitHub Releases, czym różnią się kanały Stable i Pre-releases, jak weryfikuje podpisy i jak aktualizować ręcznie.
group: run
order: 11
---

DeckPilot aktualizuje się z GitHub Releases, czyli z tego samego miejsca, w którym publikowane są instalatory. Żaden inny serwer nie bierze w tym udziału.

## Jak wygląda sprawdzanie

Otwarta aplikacja konfiguracyjna pyta GitHub o nowszą wersję, domyślnie raz dziennie. Gdy działa sam agent, sprawdzanie się nie odbywa.

Gdy istnieje nowsza wersja, DeckPilot Cię o tym informuje. Co stanie się dalej, zależy od Ciebie.

## Ustawienia

Otwórz **Settings › Updates**.

| Ustawienie | Opcje |
| --- | --- |
| Check for updates | Sprawdza od razu |
| When an update is available | **Notify me** (domyślnie), **Download, then ask** albo **Install when idle** |
| Update channel | **Stable** (domyślnie) albo **Pre-releases** |
| Check every | 6 godzin, 12 godzin, 1 dzień (domyślnie), 3 dni albo 7 dni |
| Check automatically while DeckPilot is open | Domyślnie włączone; wyłącz, aby sprawdzać tylko ręcznie |

Opcja **Install when idle** instaluje aktualizację po dziesięciu minutach bez aktywności na urządzeniu. Jest oznaczona jako niezalecana, dopóki aktualizacje nie zostaną sprawdzone w Twoim systemie; automatyczna instalacja jest wyłączona, dopóki jej nie wybierzesz.

Aktualizację stosuje przycisk **Install and restart**. Przed aktualizacją DeckPilot czeka na zakończenie działających przepływów.

## Kanały

- **Stable** oferuje wyłącznie wydania stabilne. Wydanie wstępne nigdy nie jest oferowane w tym kanale.
- **Pre-releases** oferuje najnowsze opublikowane wydanie dowolnego rodzaju, więc przejdziesz na wydanie stabilne, gdy tylko będzie ono nowsze niż Twoje wydanie wstępne.

> Kanałem domyślnym jest Stable. Zainstalowane wydanie wstępne otrzyma propozycję wydania stabilnego, gdy tylko pojawi się nowsze od niego, na każdym z kanałów. Aby nadal otrzymywać także wydania wstępne, wybierz **Pre-releases**.

DeckPilot nigdy nie instaluje wersji starszej niż zainstalowana. Po powrocie do kanału Stable zainstalowane wydanie wstępne pozostaje, dopóki nie pojawi się nowsze wydanie stabilne.

## Co jest weryfikowane

Każdy pakiet aktualizacji jest podpisany (minisign). Przed instalacją aplikacja sprawdza podpis kluczem publicznym wbudowanym w program oraz to, czy podpisana wersja jest tą, którą ogłoszono. Pakiet, którego podpisu nie da się zweryfikować, jest odrzucany, a pakiety są przyjmowane wyłącznie z plików wydań tego projektu w serwisie GitHub.

## W poszczególnych systemach

- **macOS**: aplikacja sama instaluje podpisany pakiet aktualizacji.
- **Windows**: instalację przejmuje instalator systemowy. Ta ścieżka nie jest jeszcze objęta testami automatycznymi.
- **Linux**: DeckPilot aktualizuje ten sam rodzaj pakietu, który jest zainstalowany (AppImage, `.deb` albo `.rpm`). Pakiety `.deb` i `.rpm` instalują systemowe narzędzia do obsługi pakietów, które proszą o uprawnienia administratora.

## Aktualizacja ręczna

Zawsze możesz pobrać nowszy instalator ze [strony pobierania](../../download/) i zainstalować go na istniejącej wersji. Najpierw wykonaj zaszyfrowany eksport ([Pierwsze uruchomienie](../first-launch/#kopia-zapasowa-zaszyfrowany-eksport)).

## Czego jeszcze nie sprawdzono

Aktualizacja między dwoma opublikowanymi wydaniami nie została jeszcze przeprowadzona od początku do końca na rzeczywistych instalacjach; mechanizm obejmują testy automatyczne. Dlatego automatyczna instalacja pozostaje domyślnie wyłączona.
