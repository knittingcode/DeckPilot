---
title: Pierwsze uruchomienie
description: Co dzieje się przy pierwszym uruchomieniu DeckPilota, jak chroniona jest zaszyfrowana konfiguracja oraz jak ją blokować i tworzyć kopie.
group: start
order: 2
---

## Co dzieje się przy pierwszym uruchomieniu

Przy pierwszym uruchomieniu DeckPilot tworzy konfigurację, nie pytając o hasło:

1. Agent generuje losowy klucz o długości 256 bitów.
2. Zapisuje klucz w magazynie poświadczeń systemu: w pęku kluczy logowania w systemie macOS, w Menedżerze poświadczeń w systemie Windows albo w usłudze Secret Service w systemie Linux (GNOME Keyring, KDE Wallet).
3. Tworzy zaszyfrowaną bazę konfiguracji.

Jeśli magazyn poświadczeń zapyta, czy DeckPilot może z niego korzystać, zezwól na to. Gdy magazyn jest niedostępny, okno **Welcome to DeckPilot** proponuje **Try again** albo **Use a password instead**; przy wyborze hasła kończysz przyciskiem **Create encrypted configuration**.

Konfiguracja jest zawsze szyfrowana za pomocą SQLCipher (AES-256). Trybu bez szyfrowania nie ma.

## Klucz systemowy czy hasło

Sposób odblokowywania konfiguracji wybierasz w **Settings › Security**, w części **Unlock method**:

- **Klucz systemowy** (domyślnie): klucz znajduje się w magazynie poświadczeń, więc DeckPilot otwiera się bez hasła.
- **Hasło**: klucz jest wyprowadzany z Twojego hasła algorytmem Argon2id. Hasła nie da się odzyskać; trzymaj zaszyfrowany eksport jako kopię zapasową.

Przełączasz się przyciskami **Use a password instead…** i **Use the system key instead…**.

## Blokowanie

Część **Settings › Security › Locking and authentication** określa, kiedy DeckPilot się blokuje i kiedy prosi o uwierzytelnienie.

- **Lock automatically**: nigdy, po uśpieniu komputera, po zablokowaniu ekranu lub uśpieniu komputera, po 15 minutach bezczynności albo po godzinie bezczynności.
- **Ask for authentication**: tylko przy operacjach wrażliwych, po aktualizacjach i przy operacjach wrażliwych albo przy każdym uruchomieniu DeckPilota.
- **Lock now** blokuje od razu.

Gdy DeckPilot jest zablokowany, Stream Deck pokazuje napis „Locked” i nie wykonuje żadnych akcji.

Warto wiedzieć:

- Jedynym uwierzytelnianiem systemowym jest Touch ID w systemie macOS. Windows Hello i polkit nie są zintegrowane.
- Przy kluczu systemowym w systemach Windows i Linux automatyczne blokowanie i ostrzejsze zasady uwierzytelniania są niedostępne, a operacje wrażliwe wymagają jedynie potwierdzenia.
- Blokowanie razem z blokadą ekranu jest wykrywane w systemach macOS i Linux, w systemie Windows nie.
- Przy kluczu systemowym blokada jest barierą w interfejsie użytkownika, a nie zabezpieczeniem kryptograficznym. Jeśli konfiguracja ma być nieczytelna bez hasła, użyj hasła.

## Kopia zapasowa: zaszyfrowany eksport

Eksport to Twoja kopia zapasowa i sposób na przeniesienie konfiguracji na inny komputer.

1. Otwórz **Settings › Backup & Restore**.
2. W części **Export and import** wybierz **Export…**, nadaj eksportowi własne hasło, a potem **Choose file and export**. Powstaje plik `.deckpilot`.

Eksport nie zawiera ścieżek ani innych ustawień tego komputera, kluczy i haseł. Aby przywrócić konfigurację, wybierz **Import…**, wskaż plik, **Show contents**, przejrzyj import (**Add to my configuration** albo **Replace my profiles**) i potwierdź przyciskiem **Import**.

## Pierwsze kroki

1. Podłącz Stream Decka MK.2. Pojawi się w **Settings › Devices** ([Urządzenia](../devices/)).
2. Utwórz profil i dodaj swoje projekty ([Profile i projekty](../profiles/)).
3. Utwórz akcje z potrzebnych modułów ([Akcje](../actions/)).
4. Umieść je na klawiszach w narzędziu Layout Designer ([Układy](../layouts/)).
5. Gdy jedna akcja nie wystarcza, połącz kilka w przepływ ([Przepływy](../workflows/)).
6. Wykonaj zaszyfrowany eksport.
