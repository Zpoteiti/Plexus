import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { useAuthStore } from './store/auth'
import Login from './pages/Login'
import Chat from './pages/Chat'
import Settings from './pages/Settings'
import Admin from './pages/Admin'
import Wizard, { WIZARD_KEY } from './pages/Wizard'

function RequireAuth({ children }: { children: React.ReactNode }) {
  const token = useAuthStore(s => s.token)
  if (!token) return <Navigate to="/login" replace />
  return <>{children}</>
}

function RequireAdmin({ children }: { children: React.ReactNode }) {
  const token = useAuthStore(s => s.token)
  const isAdmin = useAuthStore(s => s.isAdmin)
  if (!token) return <Navigate to="/login" replace />
  if (!isAdmin) return <Navigate to="/chat" replace />
  return <>{children}</>
}

// Redirect all users without a display name, and admins who haven't finished setup
function RequireSetup({ children }: { children: React.ReactNode }) {
  const token = useAuthStore(s => s.token)
  const isAdmin = useAuthStore(s => s.isAdmin)
  const displayName = useAuthStore(s => s.displayName)
  if (!token) return <Navigate to="/login" replace />
  // Everyone must set a name first
  if (!displayName) return <Navigate to="/setup" replace />
  // Admins must also complete the full setup wizard
  if (isAdmin && !localStorage.getItem(WIZARD_KEY)) return <Navigate to="/setup" replace />
  return <>{children}</>
}

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/" element={<Navigate to="/chat" replace />} />
        <Route path="/chat" element={<RequireSetup><Chat /></RequireSetup>} />
        <Route path="/chat/:sessionId" element={<RequireSetup><Chat /></RequireSetup>} />
        <Route path="/settings" element={<RequireAuth><Settings /></RequireAuth>} />
        <Route path="/admin" element={<RequireAdmin><Admin /></RequireAdmin>} />
        <Route path="/setup" element={<RequireAuth><Wizard /></RequireAuth>} />
        <Route path="*" element={<Navigate to="/chat" replace />} />
      </Routes>
    </BrowserRouter>
  )
}
