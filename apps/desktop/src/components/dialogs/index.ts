/**
 * DeckPilot's dialog system: one frame (`ModalShell`), the templates built
 * on it, and the service that opens them from code. docs/dialogs.md.
 */
export {
  ModalIcon,
  ModalShell,
  type ModalAction,
  type ModalShellProps,
} from "@/components/modal-shell"
export { FieldError } from "@/components/field-error"
export { CredentialDialog } from "./credential-dialog"
export {
  AlertDialog,
  ConfirmDialog,
  DestructiveDialog,
  FormDialog,
  PermissionDialog,
  ProgressDialog,
  SelectionDialog,
  type ConfirmDialogProps,
  type SelectionOption,
} from "./templates"
export { DialogProvider, type Dialogs } from "./dialog-service"
export { useDialogs } from "./use-dialogs"
