import { create } from "zustand"
import { persist } from "zustand/middleware"
import type { Page, Snapshot } from "@/lib/model"

export type Theme = "light" | "dark" | "system"
export const initialNav: Page[] = ["overview", "notifications", "links", "transfers", "settings", "about"]

interface UiStore {
  page: Page
  navOrder: Page[]
  theme: Theme
  search: string
  collapsed: boolean
  snapshot: Snapshot | null
  busy: boolean
  notice: { message: string; error: boolean } | null
  setPage: (page: Page) => void
  setTheme: (theme: Theme) => void
  setSearch: (search: string) => void
  setCollapsed: (collapsed: boolean) => void
  setNavOrder: (order: Page[]) => void
  setSnapshot: (snapshot: Snapshot) => void
  setBusy: (busy: boolean) => void
  setNotice: (message: string | null, error?: boolean) => void
}

export const useUi = create<UiStore>()(
  persist(
    (set) => ({
      page: "overview",
      navOrder: initialNav,
      theme: "system",
      search: "",
      collapsed: false,
      snapshot: null,
      busy: false,
      notice: null,
      setPage: (page) => set({ page, search: "" }),
      setTheme: (theme) => set({ theme }),
      setSearch: (search) => set({ search }),
      setCollapsed: (collapsed) => set({ collapsed }),
      setNavOrder: (order) => set({ navOrder: order }),
      setSnapshot: (snapshot) => set({ snapshot }),
      setBusy: (busy) => set({ busy }),
      setNotice: (message, error = false) => set({ notice: message ? { message, error } : null }),
    }),
    {
      name: "conduit-nova-visual-settings",
      partialize: (state) => ({
        navOrder: state.navOrder,
        theme: state.theme,
        collapsed: state.collapsed,
      }),
      version: 1,
    },
  ),
)
