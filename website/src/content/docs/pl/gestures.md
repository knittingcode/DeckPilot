---
title: Gesty
description: Nadaj klawiszowi do trzech znaczeń dzięki pojedynczemu i podwójnemu kliknięciu oraz długiemu naciśnięciu, dostrój czasy i poznaj klawisz Work Session.
group: design
order: 6
---

Klawisz może zrobić jedną rzecz w chwili naciśnięcia albo do trzech rzeczy, zależnie od sposobu naciśnięcia.

## Klawisze natychmiastowe

Klawisz, który ma tylko pojedyncze kliknięcie, działa od razu po naciśnięciu: **Single click (immediate)**. Taki klawisz może też zrobić coś przy zwolnieniu: **Release (immediate)**.

## Trzy gesty

W narzędziu Device Editor zaznacz klawisz i otwórz część **Gestures** w panelu właściwości. Znajdują się tam trzy wiersze: **Single click**, **Double click** i **Long press**. Każdy ma cel (akcję, przepływ albo krok nawigacji), opcjonalną etykietę oraz opcję **Override parameters for this gesture**, która uruchamia tę samą akcję z innymi wartościami.

Dodanie podwójnego kliknięcia albo długiego naciśnięcia zmienia zachowanie klawisza, dlatego DeckPilot najpierw pyta (**Switch Key N to gestures?**):

- Przypisanie do zwolnienia klawisza jest usuwane: naciśnięcia i zwolnienia nie można łączyć z gestami na jednym klawiszu.
- Przy długim naciśnięciu pojedyncze kliknięcie działa w chwili zwolnienia klawisza.
- Przy podwójnym kliknięciu pojedyncze kliknięcie czeka, aż minie okno podwójnego kliknięcia.

## Czasy

| Ustawienie | Wartość domyślna | Zakres |
| --- | --- | --- |
| Double-click window (okno podwójnego kliknięcia) | 300 ms | od 150 do 1000 ms |
| Long-press time (czas długiego naciśnięcia) | 700 ms | od 300 do 5000 ms |

Wartości domyślne dla tego komputera ustawisz w **Settings › Devices**, na karcie **Gesture timing** (**Save**, **Restore defaults**). Pojedynczy klawisz może je nadpisać w panelu właściwości; **Use default** przywraca wartości komputera.

## Pokrętła i strefy dotykowe

Edytor gestów zna też obrót pokrętła oraz dotknięcie, długie naciśnięcie i przesunięcie palcem po strefie dotykowej. Takie elementy ma tylko Stream Deck +, który nie ma sterownika, więc można je zaprojektować, ale nie da się ich używać.

## Klawisz Work Session

Work Session to pomiar czasu pracy rozpoczynany i kończony jednym klawiszem.

1. W narzędziu Device Editor zaznacz klawisz i wybierz **Work Session**, a potem **Work Session controller**.
2. Kliknij klawisz (**Start Work**), aby rozpocząć sesję. Klawisz pokazuje upływający czas.
3. Jeśli skonfigurowano przepływ weryfikujący zakończenie, kliknij klawisz: sprawdzenie zostanie wykonane, a gdy się powiedzie, klawisz zaproponuje **Finish Work**; kliknij go, aby zakończyć. Bez przepływu weryfikującego przytrzymaj klawisz (**Hold to finish**).

Strona **Configure › Work Session** pokazuje bieżącą sesję i jej historię; nie ma na niej przycisków rozpoczynania ani kończenia. W ustawieniach znajdziesz między innymi **Start workflow**, **End verification workflow**, czas ważności zatwierdzenia oraz próg, po którym przerwa (na przykład uśpienie komputera) jest oznaczana w historii. W danej chwili może trwać tylko jedna sesja.
