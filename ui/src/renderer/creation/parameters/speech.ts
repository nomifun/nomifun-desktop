/** Exact NomiFun catalog coordinate for the `speech_synthesis` task. */
interface SpeechGenerationModelIdentity {
  providerId: string;
  model: string;
}

/**
 * Product intent only. The integration adapter decides which optional values
 * the selected provider protocol can carry; this object is not an API body.
 */
export interface SpeechGenerationValue {
  text: string;
  instructions: string;
  voice: string;
  format: string;
  speed: number;
  model: SpeechGenerationModelIdentity | null;
}

export interface SpeechGenerationFieldSupport {
  voice: boolean;
  format: boolean;
  speed: boolean;
  instructions: boolean;
  references: boolean;
}
