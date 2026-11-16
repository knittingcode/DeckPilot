---
title: Akcje
description: Twórz akcje z wbudowanych modułów, testuj je, zarządzaj poświadczeniami i zezwalaj na możliwości, które domyślnie są wyłączone.
group: automate
order: 7
---

Akcja to jedna rzecz, którą DeckPilot potrafi zrobić: otworzyć aplikację, odczytać stan repozytorium, wysłać żądanie HTTP. Potrzebne akcje tworzysz z typów akcji udostępnianych przez wbudowane moduły, a potem używasz ich na klawiszach i w przepływach. Biblioteka jest na początku pusta; nic nie powstaje automatycznie.

## Tworzenie akcji

1. Otwórz **Configure › Action Library** i wybierz **New action**.
2. **Step 1 of 2: choose what the action does.** Typy akcji są pogrupowane według modułów; pole **Search action types** je filtruje.
3. **Step 2 of 2: name it and set its parameters.** Wypełnij pola **Name**, **Icon**, **Key label** (do 32 znaków; `{project.key}` i `{project.name}` są zastępowane danymi aktywnego projektu) oraz **Description**, a potem parametry danego typu.
4. Wybierz **Create action**.

Istniejąca akcja ma przyciski **Save**, **Duplicate**, **Run** i **Delete**. **Run** testuje akcję i pokazuje wynik, komunikat oraz wartości wyjściowe. Wymaga działającego agenta i tego, by profil był profilem aktywnym. Lista **Used in** pokazuje, gdzie akcja jest używana; przy usuwaniu używanej akcji DeckPilot pyta przy każdym odwołaniu, czy je zastąpić, czy odpiąć.

Typy akcji, które coś zmieniają, przed zapisaniem pokazują informację **This action changes things**.

## Wbudowane moduły

| Moduł | Typy akcji |
| --- | --- |
| General | Log message, Wait |
| Projects | Cycle projects, Select project, Current project, Project repository, Project documentation, Resolve project resource |
| System | Open application, Open URL, Open file, Open folder, Run command, Program status |
| Workspace | Open in editor, Open pages, Open project workspace |
| Git | Repository status, Current branch, Changed files, Last commit, Remote status, Open repository |
| Docker | Docker status, Containers, Compose status, Compose up, Compose down, Compose restart, Container logs |
| HTTP | HTTP request, Health check |
| GitHub | Current GitHub user, GitHub repository, Open pull requests, Review requests, Checks, Open on GitHub |
| GitLab | Current GitLab user, GitLab project, Open merge requests, Review requests, Pipeline status, Open on GitLab |
| Jira | Current Jira user, Jira issue, Jira issues, Open in Jira |
| Notifications | Show notification, Workflow summary |
| Work Session | Brak typów akcji; moduł ma własną stronę i klawisz ([Gesty](../gestures/#klawisz-work-session)) |

Czego moduły celowo nie robią:

- **Git** tylko odczytuje. Nigdy nie wykonuje operacji commit, reset, clean, checkout, push ani fetch.
- **GitHub**, **GitLab** i **Jira** tylko odczytują, żądaniami GET. GitHub Enterprise, GitLab na własnym serwerze i Jira Data Center są obsługiwane przez ustawienia modułu.
- **Docker** korzysta z programu Docker CLI z Compose v2. Compose down nigdy nie usuwa wolumenów ani obrazów.
- **Open URL** przyjmuje wyłącznie adresy `http`, `https` i `mailto`.

## Włączanie i wyłączanie modułów

W **Settings › Extensions › Modules** każdy moduł ma przełącznik. Wyłączenie modułu ukrywa jego strony i typy akcji; używające go akcje, przepływy i układy zostają zachowane i są oznaczone jako **Unavailable**, dopóki nie włączysz go ponownie. Modułu nie da się wyłączyć, gdy zależy od niego inny włączony moduł.

Wszystkie moduły są wbudowane w DeckPilota. Instalowanie modułów z innych źródeł nie jest obsługiwane; o eksperymentalnym systemie wtyczek przeczytasz na stronie [Wtyczki](../plugins/).

## Co jest wyłączone, dopóki na to nie zezwolisz

Wszystko, co może zmienić Twój system albo sięgnąć w głąb Twojej sieci, jest wyłączone na każdym komputerze, dopóki tego tam nie włączysz. Przełączniki znajdują się w ustawieniach poszczególnych modułów w **Settings › Extensions**. Nigdy nie trafiają do eksportu, a import konfiguracji nie może ich włączyć.

| Moduł | Przełącznik | Na co zezwala |
| --- | --- | --- |
| System | Allow running commands | **Run command**. Uruchamia tylko programy przypisane jako zasoby, bez powłoki. |
| Docker | Allow starting and stopping services | **Compose up**, **Compose down** i **Compose restart** |
| HTTP | Allow this computer and private networks | Żądania do tego komputera i do wskazanych hostów w sieciach prywatnych |
| GitHub, GitLab | API host may be on a private network | Własny serwer API w sieci prywatnej |
| Jira | Site may be on a private network | Witrynę Jira w sieci prywatnej |

## Poświadczenia

Tokeny i inne sekrety przechowuje się jako poświadczenia; nigdy nie wpisuje się ich w akcję.

Otwórz **Settings › Extensions › HTTP**, kartę **Credentials**, i wybierz **Add a credential**: nazwa **Name** (małe litery, cyfry, `.`, `_` i `-`, na przykład `github.token`), opis **Description** i sekret **Secret**, a potem **Store credential**.

- Poświadczenie jest przechowywane w zaszyfrowanej postaci wyłącznie na tym komputerze. Nigdy nie jest częścią przepływu ani eksportu i po zapisaniu nie jest ponownie wyświetlane; zmienisz je przyciskiem **Replace secret**.
- Akcja albo moduł odwołuje się do poświadczenia przez jego nazwę.
- Poświadczenia są wysyłane tylko przez HTTPS albo do tego komputera.
- W Jira Cloud sekret ma postać `email:api-token`; Jira Data Center używa osobistego tokenu dostępu. GitLab wymaga tokenu z zakresem `read_api`.
