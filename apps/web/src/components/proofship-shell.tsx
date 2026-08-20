import { lazy, Suspense, useEffect } from 'react'
import { ConnectionPanel } from '@/components/connection-panel'
import { StartupScreen } from '@/components/startup-screen'
import { useDaemon } from '@/lib/daemon-context'

const loadConnectedApp = () => import('@/components/proofship-app')
const ConnectedProofShipApp = lazy(() => loadConnectedApp().then((module) => ({
  default: module.ProofShipApp,
})))

export function ProofShipShell() {
  const { config, phase } = useDaemon()

  useEffect(() => {
    if (phase === 'connecting' && config) void loadConnectedApp()
  }, [config, phase])

  if (phase !== 'connected') return <ConnectionPanel />
  return (
    <Suspense fallback={<StartupScreen />}>
      <ConnectedProofShipApp />
    </Suspense>
  )
}
