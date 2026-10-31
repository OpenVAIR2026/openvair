import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ipc } from './ipc'

/** Starting OpenVAIR Desktop, in the tray only, when the user logs in. Off by default. */
export function useAutostart() {
  const queryClient = useQueryClient()
  const status = useQuery({ queryKey: ['autostart'], queryFn: ipc.autostart })
  const change = useMutation({
    mutationFn: ipc.setAutostart,
    onSuccess: on => queryClient.setQueryData(['autostart'], on),
  })
  return { on: status.data, change }
}
