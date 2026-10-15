/** Placeholder cards shown while a list loads. */
export function SkeletonGrid({ count = 8, rows = false }: { count?: number; rows?: boolean }) {
  return <div className={rows ? 'skeleton-rows' : 'grid-cards'} aria-hidden="true">
    {Array.from({ length: count }, (_, index) => <div className={rows ? 'skeleton-row' : 'skeleton-card'} key={index}>
      <span className="skeleton skeleton-icon" /><span className="skeleton-lines"><span className="skeleton skeleton-line" /><span className="skeleton skeleton-line short" /></span>
    </div>)}
  </div>;
}
