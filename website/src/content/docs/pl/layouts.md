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

## Strony i foldery

Układ ma postać drzewa stron. Pasek stron pokazuje strony jako karty, ścieżkę do bieżącej strony oraz napis **Page N of M**.

- Wpisz nazwę w polu **New folder**, aby dodać stronę.
- Folder otwiera klawisz z przypisaniem **Open** i nazwą strony. Folder bez klawisza **Back** dostaje go automatycznie na pierwszym wolnym klawiszu.
- **Page name** zmienia nazwę strony, a **Delete page** ją usuwa. Dla strony głównej żadna z tych opcji nie jest dostępna.

Panel drzewa stron przesuwa strony w górę i w dół, a polecenie **Move or copy** umieszcza stronę na najwyższym poziomie, wewnątrz innej strony albo w innym układzie. Te operacje są niedostępne, gdy masz niezapisane zmiany: najpierw je zapisz albo odrzuć.

## Wygląd klawisza

Zaznacz klawisz, do którego coś przypisano, i użyj ustawień wyglądu:

- **Background** i **Text colour**: próbki, selektor koloru albo wartość szesnastkowa oraz **Default**.
- **Text size**: Small, Medium albo Large.
- **Title position**: Auto, Top, Middle albo Bottom.
- **Show title**.
- Sam tytuł to pole **Label (optional)** przypisania, do 32 znaków.

### Ikony

**Import icon** przyjmuje pliki PNG i JPEG o rozmiarze do 2 MiB. Ikony są skalowane do najwyżej 256 pikseli i przechowywane w profilu, który mieści ich do 64. Ikonę dopasujesz opcjami **Fit**, **Fill** i **Crop**. Używanej ikony nie da się usunąć. DeckPilot nie ma wbudowanego zestawu ikon: biblioteka zawiera to, co zaimportujesz.

**Preview with project** pokazuje klawisze tak, jak wyglądają dla wybranego projektu, na przykład z kluczem projektu w etykiecie.

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
