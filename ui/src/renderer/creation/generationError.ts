export type GenerationErrorCode =
  | "catalog_loading"
  | "catalog_error"
  | "model_required"
  | "model_not_compatible"
  | "task_capability_mismatch"
  | "invalid_parameters"
  | "reference_not_owned"
  | "reference_kind_mismatch"
  | "reference_contract_mismatch"
  | "busy"
  | "task_not_found"
  | "task_not_retryable"
  | "disposed"
  | "presentation_state_unsupported";

export class GenerationError extends Error {
  readonly code: GenerationErrorCode;
  readonly field: string | null;

  constructor(
    code: GenerationErrorCode,
    message: string,
    field: string | null = null,
  ) {
    super(message);
    this.name = "GenerationError";
    this.code = code;
    this.field = field;
  }
}
