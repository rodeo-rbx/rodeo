// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	site: 'https://rodeo-rbx.github.io',
	base: '/rodeo/',
	integrations: [
		starlight({
			title: 'rodeo',
			logo: {
				src: './src/assets/logo.png',
				replacesTitle: true,
			},
			customCss: ['./src/styles/custom.css'],
			components: {
				ThemeSelect: './src/components/EmptyThemeSelect.astro',
			},
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/rodeo-rbx/rodeo' },
			],
			sidebar: [
				{
					label: 'Getting started',
					items: [
						{ label: 'Installation', link: '/getting-started/installation/' },
						{ label: 'CLI usage', link: '/getting-started/cli-usage/' },
						{ label: 'Directives', link: '/getting-started/directives/' },
						{ label: 'Bundling', link: '/getting-started/bundling/' },
						{ label: 'Runtime usage', link: '/getting-started/runtime-usage/' },
						{ label: 'Profiling', link: '/getting-started/profiling/' },
						{ label: 'Prebaking', link: '/getting-started/prebaking/' },
					],
				},
				{ label: 'CLI reference', link: '/cli/' },
				{ label: '@rodeo runtime', items: [{ autogenerate: { directory: 'runtime' } }] },
			],
		}),
	],
});
