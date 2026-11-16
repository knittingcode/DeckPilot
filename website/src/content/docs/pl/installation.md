---
title: Instalacja
description: Jak zainstalować DeckPilota w systemach macOS, Windows i Linux oraz czego spodziewać się po instalatorach, w tym podpisów i reguły udev.
group: start
order: 1
---

DeckPilot jest publikowany w postaci instalatorów w GitHub Releases. [Strona pobierania](../../download/) pokazuje pliki najnowszego opublikowanego wydania wraz z rozmiarami i sumami SHA-256.

> DeckPilot jest wydaniem wstępnym i nie ma jeszcze wydania stabilnego. Jedynym obsługiwanym urządzeniem jest **Stream Deck MK.2**, a z prawdziwym urządzeniem DeckPilot był używany tylko w systemie macOS. Systemy Windows i Linux obejmują testy automatyczne.

## Czego potrzebujesz

- Stream Decka MK.2. Dla innych modeli można jedynie projektować układy (zobacz [Urządzenia](../devices/)).
- Jednego z systemów, dla których DeckPilot jest budowany:

| System | Architektura | Instalator |
| --- | --- | --- |
| macOS | Apple Silicon | `DeckPilot_<wersja>_aarch64.dmg` |
| macOS | Intel | `DeckPilot_<wersja>_x64.dmg` |
| Windows | x64 | `DeckPilot_<wersja>_x64-setup.exe` lub `DeckPilot_<wersja>_x64_en-US.msi` |
| Linux | x64 | `DeckPilot_<wersja>_amd64.AppImage` lub `DeckPilot_<wersja>_amd64.deb` |

Nie ma wersji ARM dla systemów Windows i Linux. Wydania stabilne będą zawierać także pakiet `.rpm`; wydania wstępne go nie mają.

## macOS

1. Pobierz obraz dysku dla swojego Maca: `aarch64` dla Apple Silicon, `x64` dla procesorów Intel.
2. Otwórz go i skopiuj DeckPilota do folderu Aplikacje.
3. Uruchom DeckPilota.

Wersje dla systemu macOS są podpisane certyfikatem Apple Developer ID i poświadczone (notaryzowane) przez Apple, więc Gatekeeper je akceptuje. Jeśli macOS poprosi o uprawnienie „Monitorowanie wprowadzania”, przyznaj je agentowi: dzięki niemu DeckPilot odczytuje urządzenie.

## Windows

1. Pobierz program instalacyjny (`-setup.exe`) albo pakiet MSI.
2. Uruchom go.
3. Uruchom DeckPilota.

Instalatory dla systemu Windows **nie mają jeszcze podpisu kodu**. Dlatego przy pierwszym uruchomieniu filtr Windows SmartScreen wyświetla ostrzeżenie. Zanim przejdziesz dalej, porównaj sumę SHA-256 pliku z podaną na [stronie pobierania](../../download/#verify-title). Urządzenie nie wymaga instalowania sterownika: korzysta ze sterownika HID wbudowanego w system Windows.

## Linux

Wybierz pakiet Debiana albo AppImage.

### Pakiet Debiana

```sh
sudo apt install ./DeckPilot_<wersja>_amd64.deb
```

Pakiet instaluje regułę udev, która daje zalogowanemu użytkownikowi dostęp do Stream Decka MK.2 bez uprawnień roota. Po instalacji odłącz urządzenie i podłącz je ponownie.

### AppImage

```sh
chmod +x DeckPilot_<wersja>_amd64.AppImage
./DeckPilot_<wersja>_amd64.AppImage
```

AppImage nie może zainstalować reguły udev, więc trzeba to zrobić raz samodzielnie. Pobierz z repozytorium plik [`70-deckpilot-streamdeck.rules`](https://github.com/knittingcode/deckpilot/blob/main/packaging/linux/70-deckpilot-streamdeck.rules), a następnie wykonaj:

```sh
sudo cp 70-deckpilot-streamdeck.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Odłącz urządzenie i podłącz je ponownie.

### Ikona w zasobniku

Ikona agenta w zasobniku wymaga obsługi AppIndicator (StatusNotifier) w środowisku graficznym. Tam, gdzie jej nie ma, na przykład w GNOME bez rozszerzenia, ikona się nie pojawia, ale agent działa, a aplikację można otworzyć w zwykły sposób.

## Po instalacji

Uruchom DeckPilota. Agent działający w tle startuje razem z nim i pojawia się w zasobniku systemowym. Przejdź do strony [Pierwsze uruchomienie](../first-launch/).

## Gdzie DeckPilot przechowuje dane

| System | Folder |
| --- | --- |
| macOS | `~/Library/Application Support/io.github.knittingcode.deckpilot` |
| Windows | `%APPDATA%\io.github.knittingcode.deckpilot` |
| Linux | `~/.local/share/io.github.knittingcode.deckpilot` |

Dokładne położenie konfiguracji pokazuje aplikacja w **Settings › Storage**.
