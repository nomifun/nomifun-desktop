import { useEffect, useState } from 'react';
import { useAuth } from '@/renderer/hooks/context/AuthContext';
import { loadSalesAccess } from './salesTenantApi';

export const useSalesOwnerAccess = () => {
  const { status, user } = useAuth();
  const [isInstanceOwner, setIsInstanceOwner] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (status !== 'authenticated') {
      setIsInstanceOwner(false);
      setReady(status !== 'checking');
      return;
    }

    let active = true;
    setReady(false);
    void loadSalesAccess()
      .then((access) => {
        if (active) setIsInstanceOwner(access.isInstanceOwner);
      })
      .catch(() => {
        if (active) setIsInstanceOwner(false);
      })
      .finally(() => {
        if (active) setReady(true);
      });

    return () => {
      active = false;
    };
  }, [status, user?.id]);

  return { isInstanceOwner, ready };
};
