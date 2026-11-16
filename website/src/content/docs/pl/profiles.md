---
title: Profile i projekty
description: Profile rozdzielają niezależne konfiguracje, projekty sprawiają, że jeden klawisz działa na bieżącej pracy, a zasoby wiążą je z komputerem.
group: design
order: 4
---

## Profile

Profil to niezależny zestaw akcji, przepływów, projektów i układów, z własnymi ikonami. Profile służą do rozdzielania konfiguracji, które nie mają ze sobą nic wspólnego.

Otwórz **Configure › Profiles**.

- **New profile** tworzy profil: podaj **Name** i **Description**, a potem wybierz **Save**.
- **Make active** umieszcza profil na urządzeniu. Aktywny profil ma znacznik **On device**.
- Ikona ołówka otwiera okno **Edit profile** (nazwa i opis), ikona kopiowania duplikuje profil, a ikona kosza usuwa go po potwierdzeniu.

Po usunięciu aktywnego profilu urządzenie nie ma układu, dopóki nie uaktywnisz innego profilu.

### Edycja jednego profilu, gdy aktywny jest inny

Profil do edycji wybierasz z listy u góry paska bocznego; widnieje tam napis **Editing** i nazwa profilu. Jest on niezależny od profilu działającego na urządzeniu, więc możesz przygotowywać profil, nie zakłócając tego, który jest w użyciu.

## Projekty

Projekty należą do profilu. Każdy ma krótki klucz **Key** (na przykład `SHOP`), nazwę **Name** i zasoby **Resources**. Akcje zależne od projektu działają na aktywnym projekcie, dzięki czemu jeden klawisz otwiera edytor, repozytorium albo strony tego projektu, nad którym właśnie pracujesz.

Otwórz **Configure › Projects**.

- **New project** tworzy projekt.
- Każdy wiersz można edytować, skopiować do innego profilu albo usunąć.
- **Default project** to pierwszy projekt w kolejności albo wskazany projekt.
- **Cycle order**: przeciągnij, aby zmienić kolejność. Następny i poprzedni projekt wynikają z tej kolejności.

Karta przełączania projektów (poprzedni, następny, wybór) działa dla profilu aktywnego na urządzeniu. Przełączenie zmienia tylko to, do którego projektu odnoszą się akcje: nic nie jest pobierane z repozytorium, uruchamiane ponownie ani otwierane.

Aby przełączać projekty ze Stream Decka, utwórz akcję **Cycle projects** albo **Select project** ([Akcje](../actions/)) i umieść ją na klawiszu.

## Zasoby na tym komputerze

Projekt nazywa swoje zasoby według roli, na przykład repozytorium albo folder z dokumentacją. Ich rzeczywiste położenie różni się między komputerami, dlatego przypisanie jest przechowywane osobno na każdym komputerze.

Otwórz **Configure › Projects**, kartę **This machine**, część **Resources on this machine** i wybierz **Add mapping**:

- **Resource key**: nazwa, do której odwołują się akcje.
- **Kind**: Directory, Application, Terminal, Docker environment, Local service, URL albo Other.
- **Value**: ścieżka, program albo adres na tym komputerze.

Przypisania są przechowywane tylko na tym komputerze i nigdy nie stają się częścią profilu, dlatego profil można wyeksportować i używać gdzie indziej. Przy każdym przypisaniu widać jego stan: **Found**, **Missing**, **Not set on this machine** albo **Not checked**.
