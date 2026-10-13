# src/components/PremiumGate.tsx

- accountIsAdmin · function · L26-L28 — function accountIsAdmin(): boolean
- CheckoutPhase · type · L35-L35 — type CheckoutPhase = "select" | "pending" | "success";
- UpsellStore · interface · L48-L53 — interface UpsellStore
- openSupporterUpsell · function · L72-L79 — function openSupporterUpsell(feature?: PremiumFeature): void
- requirePremium · function · L91-L98 — function requirePremium(feature: PremiumFeature): boolean
- useSupporterUnlockToast · function · L108-L129 — function useSupporterUnlockToast(enabled = true): void
- SupporterUpsellModal · function · L136-L161 — function SupporterUpsellModal({ unlockToast = true }: { unlockToast?: boolean } = {})
- UpsellBody · function · L163-L301 — function UpsellBody({ feature, onClose, }: { feature: PremiumFeature | null; onClose: () => void; })
- handleCheckout · function · L210-L225 — async function handleCheckout()
- RestoreSupporter · function · L307-L373 — function RestoreSupporter({ open, onToggle, }: { open: boolean; onToggle: () => void; })
- handleRestore · function · L317-L337 — async function handleRestore(e: FormEvent)
- SupporterPending · function · L378-L421 — function SupporterPending({ stalled, onClose, }: { stalled: boolean; onClose: () => void; })
- SupporterSuccess · function · L425-L443 — function SupporterSuccess({ onClose }: { onClose: () => void })
- PremiumGateProps · interface · L447-L454 — interface PremiumGateProps
- PremiumGate · function · L461-L469 — function PremiumGate({ feature, children, variant = "inline" }: PremiumGateProps)
- UpsellCard · function · L471-L506 — function UpsellCard({ feature, variant, }: { feature: PremiumFeature; variant: "inline" | "page"; })
