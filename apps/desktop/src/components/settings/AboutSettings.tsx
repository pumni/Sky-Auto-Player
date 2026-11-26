import appIcon40 from '../../assets/brand/app-icon-40.png';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import { SettingsSection } from './SettingsLayout';

interface AboutSettingsProps {
  bootstrap: Bootstrap;
}

export function AboutSettings({ bootstrap }: AboutSettingsProps) {
  const buildRows = [
    { label: 'Native version', value: bootstrap.native_build.native_version, tabular: true },
    { label: 'Build commit', value: bootstrap.native_build.native_build_commit, mono: true },
    { label: 'Native ABI', value: bootstrap.native_build.native_abi },
    { label: 'Rust', value: bootstrap.native_build.rustc_version, tabular: true },
    {
      label: 'Win32 backend',
      value: bootstrap.native_build.win32_backend ? 'Enabled' : 'Disabled',
    },
  ];

  return (
    <section
      data-settings-category="about"
      data-slot="settings-category-content"
      className="grid content-start gap-4"
      aria-labelledby="about-settings-title"
    >
      <header className="flex min-w-0 items-start gap-3.5">
        <img
          className="size-10 flex-none rounded-control"
          src={appIcon40}
          alt=""
          width={40}
          height={40}
          aria-hidden="true"
        />
        <div className="grid min-w-0 gap-1">
          <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
            <h3
              className="m-0 text-lg font-semibold leading-6 tracking-tight text-foreground"
              id="about-settings-title"
            >
              About Sky Auto Player
            </h3>
            <span className="text-xs font-medium tabular-nums text-text-secondary">
              v{bootstrap.app_version}
            </span>
          </div>
          <p className="m-0 max-w-[520px] text-xs leading-[18px] text-text-secondary">
            A focused Windows workbench for preparing and playing Sky music sheets.
          </p>
        </div>
      </header>

      <SettingsSection title="Build information" divider>
        <dl className="m-0 grid divide-y divide-border">
          {buildRows.map((row) => (
            <div
              key={row.label}
              className="grid grid-cols-1 items-baseline gap-x-3 gap-y-0.5 py-2 @min-[400px]/settings-content:grid-cols-[140px_minmax(0,1fr)]"
            >
              <dt className="text-xs leading-[18px] text-text-tertiary">{row.label}</dt>
              <dd
                className={`m-0 min-w-0 text-xs leading-[18px] text-foreground ${row.mono ? 'break-all font-mono' : 'break-words'} ${row.tabular ? 'tabular-nums' : ''}`}
              >
                {row.value}
              </dd>
            </div>
          ))}
        </dl>
      </SettingsSection>
    </section>
  );
}
