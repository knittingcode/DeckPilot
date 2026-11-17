---
title: Zmienne
description: Odwołuj się jedną składnią do wartości współdzielonych, aktywnego projektu, wartości systemowych i wyników wcześniejszych kroków, a sekrety pozostają ukryte.
group: automate
order: 9
---

Zmienne przenoszą wartość z miejsca, w którym jest znana, tam, gdzie jest potrzebna: ścieżkę używaną przez wiele akcji, klucz aktywnego projektu, wynik wcześniejszego kroku.

## Składnia

Odwołanie to nazwa w podwójnych nawiasach klamrowych:

```text
{{ projects.current.key }}
cd {{ shared.paths.root }}
```

To zwykłe podstawienie. Nie ma wyrażeń, wartość nigdy nie jest rozwijana po raz drugi, a nieznana nazwa, na przykład `{{ env.home }}`, jest błędem. Odwołanie, które stanowi całą wartość, zachowuje jej typ; wewnątrz tekstu staje się tekstem.

## Do czego można się odwołać

| Nazwa | Wartość |
| --- | --- |
| `shared.<przestrzeń>.<nazwa>` | Utworzona przez Ciebie zmienna współdzielona |
| `projects.current.id`, `.key`, `.name` | Aktywny projekt |
| `system.profile.current.id`, `.name` | Aktywny profil |
| `system.device.current.id` | Bieżące urządzenie |
| `system.platform` | System operacyjny |
| `workflow.id`, `workflow.execution.id` | Wykonywany przepływ i to uruchomienie |
| `workSession.state`, `.startedAt`, `.elapsedSeconds`, `.canFinish` i inne | Bieżąca sesja Work Session |
| `steps.<węzeł>.output.<klucz>` | Wyjście wcześniejszego węzła przepływu |
| `steps.vars.output.<klucz>` | Zmienna ustawiona wcześniej w tym uruchomieniu |

## Gdzie działają odwołania

W parametrach akcji, w mapowaniach węzłów przepływu oraz w przypisaniach klawiszy i nadpisanych parametrach. Wpisz `{{` w polu, aby otworzyć listę dostępnych zmiennych, albo użyj przycisku **Insert a variable**, żeby otworzyć okno wyboru. Pokazuje ono wartość, źródło, typ i zakres każdej zmiennej w grupach Shared, System, Profile, Project, Modules, Workflow i Step outputs.

Bieżące wartości są widoczne tylko wtedy, gdy agent działa. Wartości zależne od uruchomienia, na przykład projekt danego uruchomienia, nie są znane, dopóki przepływ nie zostanie uruchomiony.

## Zmienne współdzielone

Otwórz **Settings › General › Variables** i wybierz **New variable**.

- **Key**: na przykład `paths.root`, od dwóch do czterech członów rozdzielonych kropkami. Po zapisaniu nie można go zmienić.
- **Label** i **Description**.
- **Type**: Text, Integer, Yes/no, Project, Profile, Resource, JSON albo Stored credential.
- **Scopes**: Global, Profile, Project.
- **Sensitive** oraz **Default value**.

Zmienna może mieć inną wartość w każdym zakresie; ustawia się je w części **Values**. Wartość projektu ma pierwszeństwo przed wartością profilu, wartość profilu przed globalną, a wartość domyślna jest używana na końcu.

Tabela pokazuje każdą zmienną z właścicielem, typem, zakresem, bieżącą wartością i informacją, czy można ją edytować, oraz pozwala skopiować odwołanie. Używanej zmiennej nie da się usunąć, dopóki nie znikną odwołania do niej.

## Sekrety

- Zmienna wrażliwa jest pokazywana jako ukryta. Jej wartość nie jest ponownie wyświetlana, także w dziennikach i w historii.
- Zmienna typu **Stored credential** przechowuje tylko nazwę poświadczenia, a nie sekret ([Akcje](../actions/#poświadczenia)).
- Wartość wrażliwa, która trafiłaby do niewrażliwego wyjścia, zmiennej albo wejścia, nie przechodzi walidacji, więc nie wycieknie przez przypadek.
- Poświadczenie albo zasób wewnątrz tekstu to błąd typu.
