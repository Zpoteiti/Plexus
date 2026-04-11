import { create } from 'zustand'
import { api } from '../lib/api'
import type { Device } from '../lib/types'

interface DevicesState {
  devices: Device[]
  loading: boolean
  fetchDevices: () => Promise<void>
  startPolling: () => () => void   // returns cleanup fn — call on unmount
}

export const useDevicesStore = create<DevicesState>((set, get) => ({
  devices: [],
  loading: false,

  fetchDevices: async () => {
    set({ loading: true })
    try {
      const devices = await api.get<Device[]>('/api/devices')
      set({ devices })
    } catch {
      // silently ignore — UI shows stale state
    } finally {
      set({ loading: false })
    }
  },

  startPolling: () => {
    const { fetchDevices } = get()
    void fetchDevices()
    const id = setInterval(() => void fetchDevices(), 5_000)
    return () => clearInterval(id)
  },
}))
