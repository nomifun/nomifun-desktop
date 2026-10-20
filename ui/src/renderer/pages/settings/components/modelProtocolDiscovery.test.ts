import { expect, test } from 'bun:test';
import { applyDiscoveredModelProtocol, modelProtocolDiscoverySnapshot } from './modelProtocolDiscovery';
import { capabilityInputsFromDefinition, emptyCapabilityDraft, patchCapabilityDraft, type ModelDefinitionDraft } from './providerModelAdvanced';

const definition = (): ModelDefinitionDraft => ({
  model: 'test-model', capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: 'openai.chat_text' }],
});

test('an old protocol discovery result cannot erase a later user image constraint', () => {
  const original = definition();
  const snapshot = modelProtocolDiscoverySnapshot(original.model, original.capabilities[0]);
  const selected = capabilityInputsFromDefinition(original)![0];
  const edited = { ...original, capabilities: [patchCapabilityDraft(original.capabilities[0], {
    inputConstraints: { image_input: false },
  })] };
  expect(modelProtocolDiscoverySnapshot(edited.model, edited.capabilities[0])).not.toBe(snapshot);
  expect(applyDiscoveredModelProtocol(edited, snapshot, selected)).toBe(edited);
});

test('protocol discovery cannot author, promote or clear the local task input policy', () => {
  for (const imageInput of [undefined, false, true]) {
    const original = definition();
    original.capabilities[0] = patchCapabilityDraft(original.capabilities[0], {
      inputConstraints: imageInput === undefined ? undefined : { image_input: imageInput },
    });
    const snapshot = modelProtocolDiscoverySnapshot(original.model, original.capabilities[0]);
    const selected = { ...capabilityInputsFromDefinition(original)![0], protocol: 'openai.responses',
      input_constraints: { image_input: imageInput !== true } };
    const applied = applyDiscoveredModelProtocol(original, snapshot, selected);
    const saved = capabilityInputsFromDefinition(applied)![0];
    expect(saved.protocol).toBe('openai.responses');
    if (imageInput === undefined) expect(saved).not.toHaveProperty('input_constraints');
    else expect(saved.input_constraints).toEqual({ image_input: imageInput });
  }
});
