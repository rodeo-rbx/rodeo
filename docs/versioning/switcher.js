// Docs version switcher. build.mjs injects the <select> into every version's header,
// with each option's value already resolved to that version's copy of the current
// page (or its home page when the page doesn't exist there).

const selects = document.querySelectorAll('select[data-rodeo-version]');

for (const select of selects) {
	select.addEventListener('change', () => {
		const option = select.selectedOptions[0];
		location.href = option.value + ('samePage' in option.dataset ? location.hash : '');
	});
}

// Back/forward cache restores the picked option; show this page's version again.
addEventListener('pageshow', () => {
	for (const select of selects) {
		for (const option of select.options) option.selected = option.defaultSelected;
	}
});
