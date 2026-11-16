/* Andrei OS — dashboard: time-of-day greeting and today's date. */
(function () {
  'use strict';
  var now = new Date();
  var h = now.getHours();
  var part = h < 12 ? 'morning' : h < 18 ? 'afternoon' : 'evening';

  var greeting = document.getElementById('greeting');
  if (greeting) { greeting.textContent = 'Good ' + part; }

  var today = document.getElementById('today');
  if (today) {
    today.textContent = now
      .toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' })
      .replace(/,/g, '')
      .toUpperCase();
  }
})();
