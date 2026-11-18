import type { Dictionary } from './index';

/**
 * Polski. Ma dokładnie kształt słownika angielskiego (sprawdza to
 * kompilator). Nazwy ekranów i przycisków aplikacji zostają po angielsku,
 * bo interfejs DeckPilota jest tylko w tym języku.
 */
export const pl: Dictionary = {
  site: {
    name: 'DeckPilot',
    tagline: 'Otwartoźródłowa aplikacja do sterowania Stream Deckiem i automatyzacji pracy.',
    description:
      'DeckPilot to otwartoźródłowa, działająca lokalnie aplikacja do sterowania Stream Deckiem i automatyzacji pracy w systemach macOS, Windows i Linux. Projektuj układy, twórz akcje i przepływy, a wszystko pozostaje zaszyfrowane na Twoim komputerze.',
  },

  a11y: {
    skipToContent: 'Przejdź do treści',
    mainNav: 'Główna',
    footerNav: 'Stopka',
    openMenu: 'Otwórz menu',
    closeMenu: 'Zamknij menu',
    homeLink: 'DeckPilot, strona główna',
    languageSwitcher: 'Język',
    switchToDark: 'Włącz ciemny motyw',
    switchToLight: 'Włącz jasny motyw',
    breadcrumb: 'Ścieżka nawigacji',
    table: 'Tabela',
    copyCode: 'Kopiuj kod',
    copied: 'Skopiowano',
    copyFailed: 'Nie udało się skopiować',
    marketplaceSite: 'otwiera witrynę Marketplace',
  },

  nav: {
    download: 'Pobierz',
    docs: 'Dokumentacja',
    screenshots: 'Zrzuty ekranu',
    plugins: 'Wtyczki',
    github: 'GitHub',
  },

  footer: {
    about:
      'DeckPilot to otwartoźródłowa, działająca lokalnie aplikacja do sterowania Stream Deckiem i automatyzacji pracy w systemach macOS, Windows i Linux.',
    product: 'Produkt',
    resources: 'Materiały',
    project: 'Projekt',
    download: 'Pobierz',
    screenshots: 'Zrzuty ekranu',
    features: 'Co potrafi',
    security: 'Bezpieczeństwo i prywatność',
    faq: 'Pytania i odpowiedzi',
    docs: 'Dokumentacja użytkownika',
    installation: 'Instalacja',
    troubleshooting: 'Rozwiązywanie problemów',
    marketplace: 'Plugin Marketplace',
    pluginDevelopers: 'Portal dla twórców wtyczek',
    github: 'DeckPilot w serwisie GitHub',
    releases: 'Wydania',
    changelog: 'Lista zmian',
    contributing: 'Zasady współpracy',
    securityPolicy: 'Polityka bezpieczeństwa',
    codeOfConduct: 'Kodeks postępowania',
    license: 'Licencja',
    legal: 'DeckPilot i ta witryna są udostępniane na licencji Apache-2.0.',
    trademark:
      'Stream Deck jest znakiem towarowym swojego właściciela. DeckPilot to niezależny projekt open source, niepowiązany z firmami Elgato i Corsair ani przez nie niewspierany.',
  },

  status: {
    prerelease: 'Wydanie wstępne',
    stable: 'Wydanie stabilne',
    noRelease: 'W przygotowaniu',
    prereleaseBadge: 'Wydanie wstępne {version}',
    stableBadge: 'Wersja {version}',
  },

  home: {
    title: 'DeckPilot: Twój Stream Deck, Twój sposób pracy',
    description:
      'DeckPilot to otwartoźródłowa aplikacja dla systemów macOS, Windows i Linux, która konfiguruje Stream Decka i automatyzuje pracę za pomocą akcji i przepływów. Lokalnie i w zaszyfrowanej postaci.',
    hero: {
      headline: 'Twój Stream Deck. Twój sposób pracy.',
      lead: 'DeckPilot to otwartoźródłowa, wieloplatformowa aplikacja do konfigurowania urządzeń Stream Deck i automatyzowania zadań za pomocą akcji i przepływów. Rozszerzają ją wtyczki z podpisanego Marketplace, działające w piaskownicy.',
      download: 'Pobierz DeckPilota',
      plugins: 'Przeglądaj wtyczki',
      github: 'Zobacz w serwisie GitHub',
      note: 'Bezpłatny i otwarty, na licencji Apache-2.0. Bez konta, bez chmury, bez telemetrii.',
    },
    overview: {
      eyebrow: 'O produkcie',
      title: 'Pulpit sterowania Twoją własną pracą',
      lead: 'Zbuduj bibliotekę akcji (otwórz aplikację, sprawdź repozytorium, wywołaj API, przełącz projekt), łącz je w przepływy i umieszczaj na klawiszach. Profile rozdzielają niezależne konfiguracje, a dzięki projektom ten sam klawisz działa na tym projekcie, nad którym właśnie pracujesz.',
      parts: [
        {
          title: 'Agent działający w tle',
          text: 'Komunikuje się z urządzeniem, rysuje klawisze, wykonuje akcje i przepływy, a jego ikona znajduje się w zasobniku systemowym. Działa także wtedy, gdy okno jest zamknięte.',
        },
        {
          title: 'Aplikacja konfiguracyjna',
          text: 'Tu projektujesz układy, akcje, przepływy i ustawienia. Łączy się z agentem, a otwierać i zamykać można ją w dowolnej chwili.',
        },
      ],
      partsLabel: 'DeckPilot składa się z dwóch części',
      points: [
        {
          title: 'Najpierw akcja',
          text: 'Najpierw ustalasz, co ma się wydarzyć, a dopiero potem, który klawisz to uruchomi. Jedna akcja może trafić na wiele klawiszy, układów i gestów.',
        },
        {
          title: 'Zbudowany wokół projektów',
          text: 'Przełączaj projekty klawiszem; akcje zależne od projektu podążają za aktywnym. Ścieżki przypisuje się osobno na każdym komputerze, więc profile pozostają przenośne.',
        },
        {
          title: 'Prawdziwa automatyzacja',
          text: 'Wizualny edytor przepływów z warunkami, gałęziami równoległymi, pętlami, ponowieniami i limitami czasu. Przepływy wykonuje agent, także przy zamkniętej aplikacji.',
        },
        {
          title: 'Prywatność w założeniach',
          text: 'Konfiguracja jest lokalna i zawsze zaszyfrowana. Wszystko, co może zmienić Twój system, jest wyłączone, dopóki nie zezwolisz na to na danym komputerze.',
        },
      ],
    },
    platforms: {
      eyebrow: 'Obsługiwane platformy',
      title: 'Jeden kod dla systemów macOS, Windows i Linux',
      lead: 'Instalatory są publikowane dla wszystkich trzech systemów, ale nie zostały sprawdzone w tym samym stopniu: z prawdziwym Stream Deckiem DeckPilot był używany tylko w systemie macOS, a systemy Windows i Linux obejmują testy automatyczne.',
      items: [
        {
          name: 'macOS',
          arch: 'Apple Silicon i Intel',
          formats: 'Obraz dysku (.dmg)',
          signing: 'Podpisany certyfikatem Developer ID i poświadczony (notaryzowany) przez Apple',
          verification: 'Sprawdzony na prawdziwym urządzeniu',
          verified: true,
        },
        {
          name: 'Windows',
          arch: 'x64',
          formats: 'Program instalacyjny (.exe) lub pakiet MSI',
          signing: 'Jeszcze bez podpisu kodu: przy pierwszym uruchomieniu ostrzega filtr SmartScreen',
          verification: 'Tylko testy automatyczne',
          verified: false,
        },
        {
          name: 'Linux',
          arch: 'x64',
          formats: 'AppImage lub pakiet .deb',
          signing: 'Pakiety nie są podpisane; pakiet .deb instaluje regułę udev dla urządzenia',
          verification: 'Tylko testy automatyczne',
          verified: false,
        },
      ],
      labels: { arch: 'Architektura', formats: 'Instalator', signing: 'Podpis', verification: 'Weryfikacja' },
      cta: 'Zobacz pliki do pobrania',
    },
    devices: {
      eyebrow: 'Obsługa urządzeń Stream Deck',
      title: 'Dziś jeden obsługiwany model, mówimy to wprost',
      lead: 'Sterownik ma wyłącznie Stream Deck MK.2. Pozostałe modele istnieją w DeckPilocie tylko jako układy, które można zaprojektować z wyprzedzeniem; nie da się ich podłączyć.',
      columns: { model: 'Model', controls: 'Elementy sterujące', status: 'Stan', detail: 'Co to oznacza' },
      rows: [
        {
          model: 'Stream Deck MK.2',
          controls: '15 klawiszy (5 × 3)',
          status: 'Obsługiwany',
          supported: true,
          detail: 'Sterownik w zestawie. Sprawdzony na prawdziwym urządzeniu w systemie macOS; w systemach Windows i Linux tylko testy automatyczne.',
        },
        {
          model: 'Stream Deck XL',
          controls: '32 klawisze',
          status: 'Tylko projektowanie układu',
          supported: false,
          detail: 'Brak sterownika. Można zaprojektować układ, ale urządzenia nie da się używać.',
        },
        {
          model: 'Stream Deck +',
          controls: '8 klawiszy, 4 pokrętła, 4 strefy dotykowe',
          status: 'Tylko projektowanie układu',
          supported: false,
          detail: 'Brak sterownika. Można zaprojektować układ, ale urządzenia nie da się używać.',
        },
      ],
      caption: 'Modele Stream Deck i stan ich obsługi w DeckPilocie',
      note: 'Praca z kilkoma urządzeniami jednocześnie nie była testowana na prawdziwym sprzęcie. Sterowniki kolejnych modeli są w planach, bez określonego terminu.',
      more: 'Podłączanie urządzenia',
    },
    layouts: {
      eyebrow: 'Profile i układy',
      title: 'Osobne konfiguracje, projektowane klawisz po klawiszu',
      lead: 'Profil to niezależny zestaw akcji, przepływów, projektów i układów. Układ należy do modelu urządzenia i można go zaprojektować bez podłączonego urządzenia.',
      items: [
        {
          title: 'Profile',
          text: 'Oddziel konfigurację do pracy, prywatną i eksperymentalną. Jeden profil możesz edytować, gdy inny działa na urządzeniu.',
        },
        {
          title: 'Projekty i zasoby',
          text: 'Projekty wybiera się w cyklu; przełączenie nigdy nie zmienia gałęzi Gita ani niczego nie zatrzymuje. Każdy komputer przypisuje zasoby projektu do własnych folderów i aplikacji.',
        },
        {
          title: 'Strony i foldery',
          text: 'Układ ma postać drzewa stron. Foldery otwiera się klawiszem, a klawisz powrotu pojawia się w nich automatycznie.',
        },
        {
          title: 'Wygląd klawiszy',
          text: 'Kolor tła i tekstu, rozmiar i położenie tytułu oraz własne ikony PNG lub JPEG. Klawisze pokazują stan na żywo.',
        },
        {
          title: 'Trzy gesty na klawisz',
          text: 'Klawisz może reagować na pojedyncze kliknięcie, podwójne kliknięcie i długie naciśnięcie, a czasy ustawisz dla komputera lub dla pojedynczego klawisza.',
        },
        {
          title: 'Edytor, który wybacza',
          text: 'Przeciągaj akcje na klawisze, kopiuj i wklejaj, cofaj i ponawiaj, a niezapisane zmiany możesz odrzucić.',
        },
      ],
      more: 'Profile, układy i gesty w dokumentacji',
    },
    actions: {
      eyebrow: 'Akcje i przepływy',
      title: 'Od jednego naciśnięcia do całej procedury',
      lead: 'Typów akcji dostarcza dwanaście wbudowanych modułów; z nich tworzysz akcje, których potrzebujesz. Gdy jedna akcja nie wystarcza, połącz kilka w przepływ.',
      modulesTitle: 'Wbudowane moduły',
      modules: [
        { name: 'General', text: 'Zapis komunikatu w dzienniku, oczekiwanie' },
        { name: 'Projects', text: 'Przełączanie i wybór projektów, odczyt ich zasobów' },
        { name: 'System', text: 'Otwieranie aplikacji, stron, plików i folderów; uruchamianie poleceń (wyłączone do czasu zezwolenia)' },
        { name: 'Workspace', text: 'Otwieranie projektu w edytorze i jego stron' },
        { name: 'Git', text: 'Tylko odczyt: stan, gałąź, zmiany, ostatni commit, stan zdalnego repozytorium' },
        { name: 'Docker', text: 'Kontenery, stan i dzienniki Compose; uruchamianie, zatrzymywanie i restart (wyłączone do czasu zezwolenia)' },
        { name: 'HTTP', text: 'Żądania z nagłówkami, parametrami i treścią JSON lub tekstową; sprawdzanie dostępności' },
        { name: 'GitHub', text: 'Tylko odczyt: pull requesty, prośby o recenzję, wyniki sprawdzeń' },
        { name: 'GitLab', text: 'Tylko odczyt: merge requesty, prośby o recenzję, pipeline’y' },
        { name: 'Jira', text: 'Tylko odczyt: zgłoszenia i wyszukiwania' },
        { name: 'Notifications', text: 'Powiadomienia na pulpicie, podsumowania przepływów' },
        { name: 'Work Session', text: 'Rozpoczynanie, sprawdzanie i kończenie mierzonych sesji pracy z klawisza' },
      ],
      workflowsTitle: 'Przepływy',
      workflows: [
        'Wizualny edytor grafu: warunki, gałęzie równoległe, pętle, przepływy podrzędne, opóźnienia',
        'Ponowienia i limity czasu dla każdego kroku oraz ścieżka błędu, gdy coś się nie powiedzie',
        'Uruchamianie z klawisza, według harmonogramu albo po zdarzeniu, na przykład podłączeniu urządzenia',
        'Postęp widoczny na żywo w edytorze i historia uruchomień, którą można przejrzeć',
        'Wynik jednego kroku trafia do następnego, a zmienne współdzielone przechowują wartości używane wszędzie',
      ],
      more: 'Czytaj w dokumentacji',
    },
    marketplace: {
      eyebrow: 'Plugin Marketplace',
      badge: 'Nowość',
      title: 'Wtyczki z podpisanego Marketplace',
      lead: 'Wtyczki pozwalają DeckPilotowi wyjść z integracjami poza wbudowane moduły: kod WebAssembly w piaskownicy, podpisany rejestr i uprawnienia, które zatwierdzasz jedno po drugim.',
      status:
        'DeckPilot instaluje wtyczki w Settings › Extensions › Plugins. Pierwsze z nich, autorstwa DeckPilota, to Air Quality, Markets, Pomodoro, Break & Hydration i Memory; integracje z GitHubem, GitLabem i Jirą oraz powiadomienia to wtyczki wbudowane. Witryna Marketplace je przedstawia i zawiera dokumentację SDK dla twórców.',
      explore: 'Otwórz Marketplace',
      developers: 'Portal dla twórców wtyczek',
      points: ['WebAssembly w piaskownicy, poza procesem aplikacji', 'Podpisany rejestr: żadnego instalowania z niesprawdzonych źródeł', 'Każde uprawnienie wymaga Twojej zgody'],
    },
    agent: {
      eyebrow: 'Agent działający w tle',
      title: 'Klawisze działają także przy zamkniętym oknie',
      lead: 'Agent to osobny, niewielki program. To on obsługuje urządzenie i wykonuje Twoje akcje i przepływy, więc aplikację konfiguracyjną otwierasz tylko wtedy, gdy chcesz coś zmienić.',
      points: [
        { title: 'Mieszka w zasobniku', text: 'Z menu w zasobniku otworzysz aplikację, zmienisz aktywny profil, zobaczysz podłączone urządzenia, odświeżysz ich listę albo zakończysz pracę agenta.' },
        { title: 'Startuje po zalogowaniu, jeśli chcesz', text: 'Jeden przełącznik w ustawieniach rejestruje agenta do uruchamiania przy logowaniu, wyłącznie dla Twojego konta.' },
        { title: 'Aplikacja i agent uzgadniają, co obsługują', text: 'Nowsza aplikacja współpracuje ze starszym agentem w zakresie, który rozumieją obie strony, i informuje, gdy agenta trzeba uruchomić ponownie.' },
        { title: 'Nic nie kończy się po cichu', text: 'Aktualizacje czekają na działające przepływy, a ponowne uruchomienie agenta wymaga potwierdzenia, zanim je przerwie.' },
      ],
      more: 'Jak działa agent w tle',
    },
    security: {
      eyebrow: 'Bezpieczeństwo i prywatność',
      title: 'Lokalnie, w zaszyfrowanej postaci i z jawnymi uprawnieniami',
      lead: 'Nie ma usługi w chmurze, konta, synchronizacji ani telemetrii. Nic nie jest nigdzie wysyłane, chyba że robi to utworzona przez Ciebie akcja. Jedyny wyjątek to sprawdzanie aktualizacji.',
      points: [
        {
          title: 'Szyfrowanie danych',
          text: 'Cała konfiguracja to baza SQLCipher (AES-256). Klucz jest przechowywany w magazynie poświadczeń systemu operacyjnego albo wyprowadzany z Twojego hasła algorytmem Argon2id.',
        },
        {
          title: 'Blokowanie',
          text: 'Blokada ręczna lub automatyczna; zablokowane urządzenie pokazuje napis „Locked” i nie wykonuje akcji. Touch ID w systemie macOS oraz ponowne uwierzytelnienie przy operacjach wrażliwych, takich jak eksport.',
        },
        {
          title: 'Jawne uprawnienia',
          text: 'Uruchamianie poleceń, uruchamianie i zatrzymywanie kontenerów oraz dostęp do sieci prywatnych i usług na własnych serwerach są wyłączone, dopóki nie zezwolisz na nie na danym komputerze.',
        },
        {
          title: 'Podpisane aktualizacje',
          text: 'Każdy pakiet aktualizacji jest podpisany, a aplikacja przed instalacją sprawdza podpis kluczem wbudowanym w program. Wersje dla systemu macOS są podpisane i poświadczone przez Apple.',
        },
        {
          title: 'Poświadczenia zostają na miejscu',
          text: 'Tokeny są przechowywane w zaszyfrowanej postaci na komputerze, w akcji wybiera się je po nazwie, nie są ponownie wyświetlane i nigdy nie trafiają do eksportu.',
        },
        {
          title: 'Szyfrowane kopie zapasowe',
          text: 'Eksport jest szyfrowany osobno, własnym hasłem. To Twoja kopia zapasowa i sposób na przeniesienie konfiguracji na inny komputer.',
        },
      ],
      caveat:
        'DeckPilot jest oprogramowaniem w wersji przed 1.0. Nie przeszedł niezależnego audytu bezpieczeństwa i nie jest przedstawiany jako gotowy do zastosowań produkcyjnych.',
      more: 'Model bezpieczeństwa i model zagrożeń',
      report: 'Zgłoś podatność prywatnie',
    },
    openSource: {
      eyebrow: 'Otwarty rozwój',
      title: 'Tworzony jawnie',
      lead: 'DeckPilot jest napisany w języku Rust, a aplikacja okienkowa korzysta z Tauri 2 oraz interfejsu w React i TypeScripcie. Cały kod źródłowy, proces wydawniczy i dokumenty projektowe są publiczne.',
      cards: [
        { title: 'Kod źródłowy', text: 'Czytaj kod, śledź postępy i zbuduj DeckPilota samodzielnie.', link: 'Otwórz repozytorium' },
        { title: 'Współpraca', text: 'Zgłoszenia błędów, pomysły i pull requesty są mile widziane. Zacznij od zasad współpracy.', link: 'Przeczytaj zasady' },
        { title: 'Plany', text: 'Sterowniki kolejnych modeli, testy sprzętowe w systemach Windows i Linux, podpisywanie kodu dla Windows.', link: 'Zobacz plany' },
        { title: 'Apache-2.0', text: 'Liberalna licencja: używaj, zmieniaj i rozpowszechniaj, z zachowaniem informacji o autorach.', link: 'Przeczytaj licencję' },
      ],
    },
    faq: {
      eyebrow: 'Pytania',
      title: 'Pytania i odpowiedzi',
      items: [
        {
          q: 'Czy DeckPilot nadaje się już do codziennej pracy?',
          a: 'DeckPilot ma już wydania stabilne, a strona pobierania oferuje najnowsze z nich. To wciąż młoda aplikacja: z prawdziwym urządzeniem była używana tylko w systemie macOS, a wersje dla Windows i Linuksa nie są podpisane cyfrowo. Trzymaj zaszyfrowany eksport swojej konfiguracji.',
        },
        {
          q: 'Które modele Stream Deck działają?',
          a: 'Tylko Stream Deck MK.2. Modele XL i Stream Deck + istnieją jako układy, które można zaprojektować, ale nie mają sterownika i nie da się ich podłączyć.',
        },
        {
          q: 'Czy potrzebuję konta albo połączenia z internetem?',
          a: 'Nie. Nie ma konta, usługi w chmurze ani telemetrii. Konfiguracja jest przechowywana w zaszyfrowanej postaci na Twoim komputerze. Otwarta aplikacja łączy się z GitHub Releases, żeby sprawdzić aktualizacje; można to wyłączyć.',
        },
        {
          q: 'Czy DeckPilot jest produktem firmy Elgato albo jest z nią powiązany?',
          a: 'Nie. DeckPilot to niezależny projekt open source. Nie jest powiązany z firmami Elgato i Corsair ani przez nie wspierany.',
        },
        {
          q: 'Czy mogę instalować wtyczki?',
          a: 'Tak: w Settings › Extensions › Plugins, z podpisanego rejestru DeckPilota. Każda wtyczka działa w piaskownicy i tylko z uprawnieniami, na które się zgodzisz.',
        },
        {
          q: 'Dlaczego Windows ostrzega mnie przy uruchamianiu instalatora?',
          a: 'Instalatory dla systemu Windows nie mają jeszcze podpisu kodu, więc filtr SmartScreen wyświetla ostrzeżenie przy pierwszym uruchomieniu. Jeśli chcesz mieć pewność, że to opublikowany plik, porównaj jego sumę SHA-256 z podaną na stronie pobierania.',
        },
        {
          q: 'Jak przenieść konfigurację na inny komputer?',
          a: 'Wykonaj zaszyfrowany eksport w ustawieniach, w sekcji Backup & Restore, i zaimportuj go na drugim komputerze. Eksport nie zawiera ścieżek ani innych ustawień danego komputera, kluczy i haseł; przypisujesz je tam ponownie.',
        },
        {
          q: 'Jak działają aktualizacje?',
          a: 'Otwarta aplikacja sprawdza GitHub Releases, domyślnie raz dziennie, i informuje, gdy jest nowsza wersja. Automatyczna instalacja jest wyłączona, dopóki jej nie wybierzesz. Aby otrzymywać wydania wstępne, wybierz kanał Pre-releases w ustawieniach, w sekcji Updates.',
        },
      ],
    },
  },

  download: {
    title: 'Pobierz DeckPilota',
    metaTitle: 'Pobierz DeckPilota dla systemów macOS, Windows i Linux',
    description:
      'Pobierz DeckPilota dla systemów macOS, Windows i Linux. Instalatory, sumy kontrolne i informacje o wydaniu pochodzą prosto z opublikowanego wydania w serwisie GitHub.',
    lead: 'Instalatory udostępnia GitHub Releases, czyli źródło nadrzędne. Ta strona pokazuje, co faktycznie zawiera najnowsze opublikowane wydanie.',
    release: {
      version: 'Wersja',
      channel: 'Kanał',
      published: 'Data publikacji',
      notes: 'Informacje o wydaniu',
      allAssets: 'Wszystkie pliki tego wydania w serwisie GitHub',
      checksums: 'SHA256SUMS',
      prereleaseTitle: 'To jest wydanie wstępne',
      prereleaseText:
        'DeckPilot nie ma jeszcze wydania stabilnego. Wydanie wstępne służy do wypróbowania aplikacji: spodziewaj się niedoróbek i trzymaj zaszyfrowany eksport swojej konfiguracji.',
      newerPrerelease: 'Dostępne jest też nowsze wydanie wstępne: {version}.',
      newerPrereleaseLink: 'Zobacz je w serwisie GitHub',
      generated: 'Dane odczytane z GitHub Releases {date}.',
    },
    fresh: {
      newer: 'Od czasu zbudowania tej strony opublikowano nowsze wydanie: {version}.',
      link: 'Pobierz je z GitHub Releases',
    },
    os: { macos: 'macOS', windows: 'Windows', linux: 'Linux' },
    arch: { arm64: 'Apple Silicon', x64: 'x64', intel: 'Intel', arm64Generic: 'ARM64' },
    format: {
      dmg: 'Obraz dysku (.dmg)',
      exe: 'Program instalacyjny (.exe)',
      msi: 'Pakiet MSI (.msi)',
      appimage: 'AppImage',
      deb: 'Pakiet Debiana (.deb)',
      rpm: 'Pakiet RPM (.rpm)',
    },
    downloadAsset: 'Pobierz',
    size: 'Rozmiar',
    sha256: 'SHA-256',
    noSha: 'Dla tego pliku nie opublikowano sumy kontrolnej.',
    signing: {
      title: 'Podpis',
      macSigned: 'Podpisany certyfikatem Apple Developer ID i poświadczony (notaryzowany) przez Apple, więc Gatekeeper go akceptuje.',
      macSignedNotNotarized: 'Podpisany certyfikatem Apple Developer ID, ale niepoświadczony przez Apple.',
      winSigned: 'Z podpisem kodu (Authenticode).',
      winUnsigned:
        'Bez podpisu kodu. Przy pierwszym uruchomieniu filtr Windows SmartScreen wyświetli ostrzeżenie: po sprawdzeniu pliku wybierz „Więcej informacji”, a potem „Uruchom mimo to”.',
      linuxUnsigned: 'Pakiet nie jest podpisany. Sprawdź pobrany plik za pomocą sumy SHA-256.',
      unknown: 'To wydanie nie zawiera informacji o sposobie podpisania tej wersji.',
    },
    notes: {
      macos: 'Otwórz obraz dysku i przeciągnij DeckPilota do folderu Aplikacje.',
      windows: 'Uruchom program instalacyjny albo zainstaluj pakiet MSI: oba instalują tę samą aplikację.',
      linux:
        'Pakiet .deb instaluje regułę udev, dzięki której Stream Decka można używać bez uprawnień roota. Przy AppImage regułę trzeba zainstalować samodzielnie (zobacz: Instalacja).',
    },
    empty: {
      title: 'Nie opublikowano jeszcze żadnego wydania',
      text: 'W tej chwili nie ma nic do pobrania. Wydania pojawią się tutaj, gdy tylko zostaną opublikowane w serwisie GitHub.',
      releases: 'Wydania w serwisie GitHub',
      source: 'Zbuduj ze źródeł',
    },
    noInstallers: 'To wydanie nie zawiera instalatorów, które rozpoznaje ta strona. Wszystkie jego pliki znajdziesz w serwisie GitHub.',
    verify: {
      title: 'Sprawdź pobrany plik',
      lead: 'Przy każdym pliku podajemy jego sumę SHA-256 w postaci, w jakiej GitHub podaje ją dla opublikowanego zasobu. Oblicz sumę pobranego pliku i porównaj obie wartości.',
      macLinux: 'macOS i Linux',
      windows: 'Windows (PowerShell)',
      sums: 'Wydanie zawiera też plik SHA256SUMS z sumami kontrolnymi wszystkich swoich plików.',
      attest: 'Wydania mają poświadczenia pochodzenia kompilacji. Za pomocą GitHub CLI można sprawdzić, że plik został zbudowany przez proces wydawniczy tego repozytorium:',
    },
    updater: {
      title: 'Czym są pozostałe pliki wydania',
      lead: 'Wydanie zawiera nie tylko instalatory. Tych plików nie potrzebujesz, żeby zainstalować DeckPilota.',
      items: [
        { name: '*.sig', text: 'Podpisy pakietów aktualizacji (minisign). Przed instalacją aktualizacji aplikacja sprawdza je kluczem publicznym wbudowanym w program.' },
        { name: '*.app.tar.gz', text: 'Pakiet aktualizacji dla systemu macOS, który aplikacja pobiera, gdy aktualizuje się sama. To nie jest instalator.' },
        { name: 'prerelease.json, latest.json', text: 'Manifest aktualizacji odczytywany przez aplikację: prerelease.json w wydaniu wstępnym, latest.json w wydaniu stabilnym.' },
        { name: 'release-meta.json', text: 'Opis wydania: tag, wersja, kanał, commit oraz sposób podpisania każdej wersji.' },
        { name: 'SHA256SUMS', text: 'Sumy SHA-256 wszystkich plików wydania.' },
      ],
      note: 'Aktualizacje są dostarczane wyłącznie przez GitHub Releases. Ta witryna nie udostępnia instalatorów ani plików aktualizacji.',
    },
    requirements: {
      title: 'Zanim zainstalujesz',
      items: [
        'Jedynym obsługiwanym urządzeniem jest Stream Deck MK.2. Z prawdziwym urządzeniem DeckPilot był testowany w systemie macOS; systemy Windows i Linux obejmują wyłącznie testy automatyczne.',
        'Dostępne są wersje dla systemów macOS (Apple Silicon i Intel), Windows x64 i Linux x64. Nie ma wersji ARM dla systemów Windows i Linux.',
        'W systemie Linux ikona w zasobniku wymaga obsługi AppIndicator (StatusNotifier); bez niej agent nadal działa.',
      ],
      install: 'Instrukcja instalacji',
    },
  },

  screenshots: {
    title: 'Zrzuty ekranu',
    metaTitle: 'Zrzuty ekranu DeckPilota',
    description:
      'DeckPilot na zrzutach ekranu: pulpit, edytor układów, profile i projekty, edytor akcji, edytor przepływów, Work Session i ustawienia.',
    lead: 'Przegląd aplikacji, ekran po ekranie.',
    pendingTitle: 'Prawdziwe zrzuty ekranu są w przygotowaniu',
    pendingText:
      'W tej galerii pokazujemy wyłącznie prawdziwe zrzuty ekranu aplikacji. Żadnego jeszcze nie opublikowano, dlatego poniższe ramki są symbolami zastępczymi i opisują, co znajdzie się na każdym zrzucie. Nic tutaj nie jest makietą.',
    placeholder: 'Zrzut ekranu nie jest jeszcze dostępny',
    placeholderBadge: 'Symbol zastępczy',
    willShow: 'Pokaże',
    items: {
      dashboard: {
        title: 'Pulpit',
        alt: 'Pulpit DeckPilota ze stanem agenta i podłączonymi urządzeniami',
        text: 'Pulpit: stan agenta działającego w tle, aktywny profil i podłączone urządzenia.',
      },
      device: {
        title: 'Layout Designer',
        alt: 'Layout Designer z siatką klawiszy Stream Decka MK.2, biblioteką akcji i panelem właściwości',
        text: 'Layout Designer: obszar urządzenia, biblioteka akcji i przepływów oraz panel właściwości klawisza.',
      },
      profiles: {
        title: 'Profile i projekty',
        alt: 'Strona profili oraz projekty profilu z kolejnością cyklu',
        text: 'Profile i należące do nich projekty, z kolejnością cyklu i zasobami przypisanymi na tym komputerze.',
      },
      actionEditor: {
        title: 'Edytor akcji',
        alt: 'Edytor akcji z parametrami akcji',
        text: 'Edytor akcji: nazwa, ikona, etykieta klawisza i parametry typu akcji.',
      },
      workflowEditor: {
        title: 'Edytor przepływów',
        alt: 'Edytor grafu przepływu z węzłami, połączeniami i panelem uruchomień',
        text: 'Edytor przepływów: graf węzłów, katalog i historia uruchomień.',
      },
      workSession: {
        title: 'Work Session',
        alt: 'Strona Work Session z bieżącą sesją i historią',
        text: 'Work Session: bieżąca sesja, jej ustawienia i historia sesji.',
      },
      settings: {
        title: 'Ustawienia i moduły',
        alt: 'Okno ustawień z listą wbudowanych modułów',
        text: 'Ustawienia: wbudowane moduły z przełącznikami, uprawnieniami i ustawieniami modułów.',
      },
    },
  },

  docs: {
    title: 'Dokumentacja',
    metaTitle: 'Dokumentacja użytkownika DeckPilota',
    description:
      'Dokumentacja użytkownika DeckPilota: instalacja, pierwsze uruchomienie, urządzenia, profile, układy, akcje, gesty, przepływy, zmienne, agent w tle, aktualizacje i rozwiązywanie problemów.',
    lead: 'Jak zainstalować DeckPilota, podłączyć Stream Decka i zbudować własne profile, układy, akcje i przepływy.',
    groups: {
      start: 'Na początek',
      design: 'Projektowanie',
      automate: 'Automatyzacja',
      run: 'Praca z DeckPilotem',
      help: 'Pomoc',
    },
    sidebarLabel: 'Strony dokumentacji',
    menu: 'Menu dokumentacji',
    onThisPage: 'Na tej stronie',
    previous: 'Poprzednia',
    next: 'Następna',
    pagination: 'Poprzednia i następna strona',
    statusTitle: 'O tej dokumentacji',
    status:
      'DeckPilot szybko się zmienia. Te strony opisują aplikację według stanu kodu źródłowego ze wskazanego niżej commita; nazwy w interfejsie zainstalowanej wersji mogą się nieznacznie różnić.',
    uiLanguage: 'Interfejs aplikacji jest dostępny wyłącznie po angielsku, dlatego nazwy ekranów i przycisków podajemy w oryginale.',
    verified: 'Sprawdzono z kodem źródłowym DeckPilota w commicie {commit}.',
    source: 'Dokumentacja techniczna w repozytorium',
    developersTitle: 'Piszesz wtyczki?',
    developers:
      'Tworzenie wtyczek opisuje witryna Marketplace: SDK, manifest, uprawnienia, testowanie i publikowanie.',
    developersLink: 'Otwórz portal dla twórców wtyczek',
  },

  notFound: {
    title: 'Nie znaleziono strony',
    description: 'Strona, której szukasz, nie istnieje w witrynie DeckPilota.',
    heading: 'Ta strona nie istnieje',
    text: 'Adres może zawierać literówkę albo strona została przeniesiona.',
    home: 'Wróć na stronę główną',
    docs: 'Otwórz dokumentację',
  },

  root: {
    title: 'DeckPilot',
  },
};
