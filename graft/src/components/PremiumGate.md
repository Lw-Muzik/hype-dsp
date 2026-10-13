# src/components/PremiumGate.tsx

- accountIsAdmin · function · L26-L28 — function accountIsAdmin(): boolean
- CheckoutPhase · type · L35-L35 — type CheckoutPhase = "select" | "pending" | "success";
- UpsellStore · interface · L48-L53 — interface UpsellStore
- openSupporterUpsell · function · L72-L79 — function openSupporterUpsell(feature?: PremiumFeature): void
- requirePremium · function · L91-L98 — function requirePremium(feature: PremiumFeature): boolean
- useSupporterUnlockToast · function · L108-L129 — function useSupporterUnlockToast(): void
- SupporterUpsellModal · function · L132-L157 — function SupporterUpsellModal()
- UpsellBody · function · L159-L297 — function UpsellBody({ feature, onClose, }: { feature: PremiumFeature | null; onClose: () => void; })
- handleCheckout · function · L206-L221 — async function handleCheckout()
- RestoreSupporter · function · L303-L369 — function RestoreSupporter({ open, onToggle, }: { open: boolean; onToggle: () => void; })
- handleRestore · function · L313-L333 — async function handleRestore(e: FormEvent)
- SupporterPending · function · L374-L417 — function SupporterPending({ stalled, onClose, }: { stalled: boolean; onClose: () => void; })
- SupporterSuccess · function · L421-L439 — function SupporterSuccess({ onClose }: { onClose: () => void })
- PremiumGateProps · interface · L443-L450 — interface PremiumGateProps
- PremiumGate · function · L457-L465 — function PremiumGate({ feature, children, variant = "inline" }: PremiumGateProps)
- UpsellCard · function · L467-L502 — function UpsellCard({ feature, variant, }: { feature: PremiumFeature; variant: "inline" | "page"; })
