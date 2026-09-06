/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import { configService } from '@/common/config/configService';
import type { AgentSource, AgentMetadata } from '@/renderer/utils/model/agentTypes';
import { useAgents } from '@/renderer/hooks/agent/useAgents';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { getAgentKey as getAgentKeyUtil } from './agentSelectionUtils';
import type { AvailableAgent, EffectiveAgentInfo } from '../types';

export type GuidAgentSelectionResult = {
  selectedAgentKey: string;
  setSelectedAgentKey: (key: string) => void;
  defaultAgentKey: string;
  selectedAgent: string;
  selectedAgentInfo: AvailableAgent | undefined;
  availableAgents: AvailableAgent[];
  currentEffectiveAgentInfo: EffectiveAgentInfo;
  getAgentKey: (agent: {
    agent_type: string;
    agent_source?: AgentSource;
    backend?: string;
    id?: string;
    agent_id?: string;
  }) => string;
  findAgentByKey: (key: string) => AvailableAgent | undefined;
  resolveAgentType: (
    agentInfo: { agent_type: string; backend?: string } | undefined
  ) => string;
  isMainAgentAvailable: (agent_type: string) => boolean;
  getEffectiveAgentType: (
    agentInfo: { agent_type: string; backend?: string } | undefined
  ) => EffectiveAgentInfo;
  refreshCustomAgents: () => Promise<void>;
  customAgentAvatarMap: Map<string, string | undefined>;
};

type UseGuidAgentSelectionOptions = {
  modelList: IProvider[];
  localeKey: string;
  resetAgentSelection?: boolean;
  preselectAgentKey?: string;
  locationKey?: string;
};

const normalizeDetectedAgent = (agent: AgentMetadata): AvailableAgent => ({
  ...agent,
  id: agent.agent_id,
  avatar: agent.agent_source === 'custom' ? agent.icon : undefined,
});

/**
 * Guid is the quick-start surface for ordinary Agents only. Agent authoring
 * and template creation live in the Agent Workbench; no saved-Agent catalog
 * is mixed into this selector.
 */
export const useGuidAgentSelection = ({
  modelList,
  resetAgentSelection,
  preselectAgentKey,
  locationKey,
}: UseGuidAgentSelectionOptions): GuidAgentSelectionResult => {
  const [selectedAgentKey, _setSelectedAgentKey] = useState<string>(() => {
    try {
      return configService.get('guid.lastSelectedAgent') || 'nomi';
    } catch {
      return 'nomi';
    }
  });

  const {
    agents: detectedAgents,
    isLoading: agentsLoading,
    revalidate,
    refreshCustomAgents,
  } = useAgents();

  const setSelectedAgentKey = useCallback((key: string) => {
    _setSelectedAgentKey(key);
    void configService.set('guid.lastSelectedAgent', key).catch((error) => {
      console.error('Failed to save selected Agent:', error);
    });
  }, []);

  const availableAgents = useMemo(
    () => detectedAgents.map(normalizeDetectedAgent),
    [detectedAgents]
  );

  const customAgents = useMemo(
    () =>
      detectedAgents.filter(
        (agent) => agent.agent_source === 'custom' && agent.available && agent.enabled
      ),
    [detectedAgents]
  );

  const customAgentAvatarMap = useMemo(
    () => new Map(customAgents.map((agent) => [agent.agent_id, agent.icon])),
    [customAgents]
  );

  const getAgentKey = useCallback(
    (agent: {
      agent_type: string;
      agent_source?: AgentSource;
      backend?: string;
      id?: string;
      agent_id?: string;
    }) => getAgentKeyUtil(agent),
    []
  );

  const findAgentByKey = useCallback(
    (key: string): AvailableAgent | undefined =>
      availableAgents.find((agent) => getAgentKey(agent) === key),
    [availableAgents, getAgentKey]
  );

  const selectedAgentInfo = useMemo(
    () => findAgentByKey(selectedAgentKey),
    [findAgentByKey, selectedAgentKey]
  );

  const selectedAgent =
    selectedAgentInfo?.agent_source === 'custom' ? 'custom' : selectedAgentKey;

  const isMainAgentAvailable = useCallback(
    (agentType: string): boolean =>
      agentType === 'nomi'
        ? modelList.length > 0
        : availableAgents.some(
            (agent) => agent.agent_type === agentType || agent.backend === agentType
          ),
    [availableAgents, modelList]
  );

  const resolveAgentType = useCallback(
    (agentInfo: { agent_type: string; backend?: string } | undefined): string =>
      agentInfo?.backend || agentInfo?.agent_type || 'nomi',
    []
  );

  const getEffectiveAgentType = useCallback(
    (agentInfo: { agent_type: string; backend?: string } | undefined): EffectiveAgentInfo => {
      const agentType = resolveAgentType(agentInfo);
      return {
        agent_type: agentType,
        isFallback: false,
        originalType: agentType,
        isAvailable: isMainAgentAvailable(agentType),
      };
    },
    [isMainAgentAvailable, resolveAgentType]
  );

  const resetHandledRef = useRef(false);
  const previousLocationKeyRef = useRef(locationKey);
  if (locationKey !== previousLocationKeyRef.current) {
    previousLocationKeyRef.current = locationKey;
    resetHandledRef.current = false;
  }

  useLayoutEffect(() => {
    if (availableAgents.length === 0 || resetHandledRef.current) return;

    if (
      preselectAgentKey &&
      availableAgents.some((agent) => getAgentKey(agent) === preselectAgentKey)
    ) {
      resetHandledRef.current = true;
      _setSelectedAgentKey(preselectAgentKey);
      void configService.set('guid.lastSelectedAgent', preselectAgentKey);
      return;
    }

    if (resetAgentSelection) {
      resetHandledRef.current = true;
      const first = availableAgents[0];
      const fallbackKey = first ? getAgentKey(first) : 'nomi';
      _setSelectedAgentKey(fallbackKey);
      void configService.set('guid.lastSelectedAgent', fallbackKey);
    }
  }, [availableAgents, getAgentKey, preselectAgentKey, resetAgentSelection]);

  useEffect(() => {
    if (agentsLoading || availableAgents.length === 0 || resetAgentSelection) return;
    if (
      preselectAgentKey &&
      availableAgents.some((agent) => getAgentKey(agent) === preselectAgentKey)
    ) {
      return;
    }

    const savedKey = configService.get('guid.lastSelectedAgent');
    if (savedKey && availableAgents.some((agent) => getAgentKey(agent) === savedKey)) {
      _setSelectedAgentKey(savedKey);
      return;
    }

    const first = availableAgents[0];
    if (!first) return;
    const fallbackKey = getAgentKey(first);
    _setSelectedAgentKey(fallbackKey);
    if (savedKey !== fallbackKey) {
      void configService.set('guid.lastSelectedAgent', fallbackKey);
    }
  }, [agentsLoading, availableAgents, getAgentKey, preselectAgentKey, resetAgentSelection]);

  const defaultAgentKey = useMemo(() => {
    const first = availableAgents[0];
    return first ? getAgentKey(first) : 'nomi';
  }, [availableAgents, getAgentKey]);

  const currentEffectiveAgentInfo = useMemo(
    () => getEffectiveAgentType(selectedAgentInfo),
    [getEffectiveAgentType, selectedAgentInfo]
  );

  const refresh = useCallback(async () => {
    await Promise.all([refreshCustomAgents(), revalidate()]);
  }, [refreshCustomAgents, revalidate]);

  return {
    selectedAgentKey,
    setSelectedAgentKey,
    defaultAgentKey,
    selectedAgent,
    selectedAgentInfo,
    availableAgents,
    currentEffectiveAgentInfo,
    getAgentKey,
    findAgentByKey,
    resolveAgentType,
    isMainAgentAvailable,
    getEffectiveAgentType,
    refreshCustomAgents: refresh,
    customAgentAvatarMap,
  };
};
