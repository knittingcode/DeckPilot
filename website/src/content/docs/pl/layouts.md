---
title: Układy
description: Zaprojektuj w narzędziu Device Editor, co pokazuje i robi każdy klawisz, korzystając ze stron, folderów, kolorów, ikon i skrótów.
group: design
order: 5
---

Układ określa, co pokazuje i co robi każdy klawisz danego modelu urządzenia. Układy należą do profilu i do modelu urządzenia, a projektować je można bez podłączonego urządzenia.

## Device Editor

DeckPilot otwiera się w narzędziu **Device Editor**; jest ono też pierwszą pozycją paska nawigacji. Urządzenie i profil wybierasz na pasku narzędzi: lista urządzeń pokazuje najpierw podłączone urządzenia, a pozostałe modele jako **Not connected**. Następnie wybierz **Layout**; gwiazdka oznacza układ aktywny dla danego modelu.

- **New layout** tworzy układ. Przełącznik **Place the profile's actions and workflows on the keys** jest domyślnie włączony i wypełnia nowy układ za Ciebie.
- **Duplicate** kopiuje układ, **Make active** umieszcza go na urządzeniu, a **Delete** go usuwa.

Edytor składa się z trzech części: obszaru urządzenia pośrodku, panelu właściwości wybranego elementu pod nim oraz biblioteki **Library** (Actions, Workflows, Navigation) po prawej stronie. Bibliotekę można poszerzyć, przeciągając jej krawędź, i zwinąć; w wąskim oknie otwiera ją przycisk **Library**.

## Umieszczanie czegoś na klawiszu

- Przeciągnij akcję albo przepływ z biblioteki na klawisz albo
- zaznacz klawisz i wybierz w panelu właściwości, albo
- kliknij klawisz prawym przyciskiem myszy i użyj polecenia **Assign**.

Menu kontekstowe zawiera też polecenia Copy, Cut, Paste, Duplicate i Clear. To, co klawisz robi po pojedynczym kliknięciu, podwójnym kliknięciu i długim naciśnięciu, opisuje strona [Gesty](../gestures/).

## Które akcje pasują do którego elementu

Nie każda akcja działa na każdym elemencie urządzenia. Akcja określa, czy uruchamia ją klawisz, pokrętło czy strefa dotykowa, i jakie zdarzenie: naciśnięcie, obrót, dotknięcie. Większość akcji działa tylko na klawiszach; Next page, Previous page, Brightness oraz akcje przełączania projektów działają też na pokrętle. Akcje wtyczek działają na klawiszach, chyba że wtyczka określa inaczej.

- Gdy zaznaczony jest jeden element, biblioteka pokazuje, co do niego pasuje. Akcje, które nie pasują, pozostają widoczne, ale wyszarzone, z powodem, na przykład *Requires a dial. The selected Stream Deck MK.2 key supports press actions only.* Przełącznik **Compatible only** je ukrywa; wybór jest zapamiętywany.
- Upuszczenie akcji na element, z którego nie może się uruchomić, zostaje odrzucone: element dostaje czerwoną obwódkę, powód jest wyświetlany i odczytywany przez czytnik ekranu, a nic nie zostaje przypisane.
- W panelu właściwości każde zdarzenie (kliknięcie, obrót w prawo, dotknięcie) proponuje tylko akcje, które mogą się z niego uruchomić; pozostałe są oznaczone **(not on this control)**.
- Przypisanie zapisane wcześniej albo zaimportowane, które nie może działać na swoim elemencie, zostaje zachowane bez zmian, ale nie jest uruchamiane. Panel właściwości podaje powód (**Not run from this control: …**), a zapisywanie wymienia takie przypisania. Zadziała znowu, gdy pojawi się zgodna akcja albo urządzenie. Nowe takie przypisanie zapisywanie odrzuca, podając powód.

## Strony i foldery

Układ ma postać drzewa stron. Pod podglądem urządzenia przyciski **Previous page** i **Next page** przechodzą między stronami najwyższego poziomu (tymi, po których poruszają się klawisze Next page i Previous page na urządzeniu), numer strony między nimi otwiera listę wszystkich stron, a **Add page** dodaje stronę najwyższego poziomu i ją pokazuje. W folderze widać tam też ścieżkę do niego i przycisk **Back**.

- Folder otwiera klawisz z przypisaniem **Open** i nazwą strony; dwukrotne kliknięcie takiego klawisza otwiera folder. Folder bez klawisza **Back** dostaje go automatycznie na pierwszym wolnym klawiszu.
- **Manage pages** otwiera ustawienia stron: **Page name** zmienia nazwę strony, a **Delete page** ją usuwa (dla strony głównej żadna z tych opcji nie jest dostępna); w folderze wybierasz też, czy zamyka się po użyciu klawisza.

Drzewo stron w **Manage pages** przesuwa strony w górę i w dół, a polecenie **Move or copy** umieszcza stronę na najwyższym poziomie, wewnątrz innej strony albo w innym układzie. Te operacje są niedostępne, gdy masz niezapisane zmiany: najpierw je zapisz albo odrzuć.

## Wygląd klawisza

Zaznacz klawisz, do którego coś przypisano, i użyj ustawień wyglądu:

- **Background** i **Text colour**: próbki, selektor koloru albo wartość szesnastkowa oraz **Default**.
- **Text size**: Small, Medium albo Large.
- **Title position**: Auto, Top, Middle albo Bottom.
- **Show title**.
- Sam tytuł to pole **Label (optional)** przypisania, do 32 znaków.

### Ikony

**Import icon** przyjmuje pliki PNG i JPEG o rozmiarze do 2 MiB. Ikony są skalowane do najwyżej 256 pikseli i przechowywane w profilu, który mieści ich do 64. Ikonę dopasujesz opcjami **Fit**, **Fill** i **Crop**. Używanej ikony nie da się usunąć. Biblioteka ikon zawiera to, co zaimportujesz. Klawisz bez zaimportowanej ikony pokazuje ikonę swojej akcji (tę, którą aplikacja pokazuje dla akcji); zaimportowana ikona ją zastępuje.

**Preview with project** pokazuje klawisze tak, jak wyglądają dla wybranego projektu, na przykład z kluczem projektu w etykiecie.

Podgląd pokazuje każdy klawisz tak jak urządzenie, rysowany tym samym rendererem: ikonę akcji, tytuł oraz, dla klawiszy z wartością na żywo, takich jak **CPU usage**, bieżącą wartość odczytaną przez agenta dla urządzenia. Wartości pochodzą od agenta, tylko dla aktywnego profilu i tylko gdy agent działa; wiersz pod nawigacją stron informuje, gdy nie są aktualne.

## Zapisywanie, cofanie i skróty

Zmiany pozostają w edytorze, dopóki nie wybierzesz **Save**; **Revert** je odrzuca, a znacznik **Unsaved** informuje, że są niezapisane zmiany. **Undo** i **Redo** obejmują ostatnie 100 kroków.

| Skrót | Działanie |
| --- | --- |
| Cmd/Ctrl + S | Zapisz |
| Cmd/Ctrl + Z | Cofnij |
| Cmd/Ctrl + Shift + Z, Cmd/Ctrl + Y | Ponów |
| Cmd/Ctrl + C, X, V | Kopiuj, wytnij, wklej |
| Cmd/Ctrl + D | Duplikuj |
| Cmd/Ctrl + A | Zaznacz wszystko |
| Delete | Wyczyść zaznaczone klawisze |
| Escape | Usuń zaznaczenie |
| Shift + kliknięcie | Zaznacz zakres |
| Klawisze strzałek | Przesuń zaznaczenie |
| Shift + Enter | Otwórz folder z zaznaczonego klawisza |
