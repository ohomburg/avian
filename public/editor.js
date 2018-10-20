(function () {
    let INSERT = Symbol("insert");
    let DELETE = Symbol("delete");

    let socket = new WebSocket("ws://" + window.location.host + "/ws");

    let footer = document.querySelector("footer");
    let editor = document.getElementById("editor");

    let text = "";
    let rev = 0;
    let init = false;
    let myEdit = false;
    let queue = [];

    function setStatus(status, editable) {
        footer.innerText = "Status: " + status;
        if (editable === true) {
            editor.disabled = false;
        } else if (editable === false) {
            editor.disabled = true;
        }
    }

    setStatus("offline", false);

    socket.onmessage = function (event) {
        setStatus("online", true);
        if (!init) {
            [rev, text] = JSON.parse(event.data);
            editor.value = text;
            init = true;
            queueReady();
        } else if (myEdit) {
            rev = JSON.parse(event.data).rev;
            queueReady();
            myEdit = false;
        } else {
            let msg = JSON.parse(event.data);
            if (msg.success === true) {
                myEdit = true;
            } else if (msg.success === false) {
                setStatus("desync (" + msg.reason + ")", false);
                socket.onmessage = console.log;
            } else {
                applyEdit(msg);
            }
        }
    };

    socket.onopen = function () {
        setStatus("waiting for data");
    };

    socket.onclose = function () {
        setStatus("disconnected", false);
    };

    socket.onerror = function (event) {
        console.error("WebSocket error:", event);
        setStatus("error", false);
    };

    function transformEdit(oldI, newI, edit) {
        // see also: editor::History::transform
        if (Math.max(oldI, newI) < edit.pos) {
            edit.pos += newI - oldI;
        } else if (Math.min(oldI, newI) <= edit.pos) {
            // TODO Transform for overlapping ranges.
            socket.disconnect();
            setStatus("client desync (not implemented)", false);
        }
    }

    function countUtf8Bytes(s) {
        return new Blob([s]).size;
    }

    function toUTF8(s) {
        return unescape(encodeURIComponent(s));
    }

    function fromUTF8(s) {
        return decodeURIComponent(escape(s));
    }

    function queueSend(data) {
        if (queue.ready) {
            socket.send(JSON.stringify(data));
            queue.ready = false;
        } else {
            queue.push(data);
        }
    }

    function queueReady() {
        if (queue.length) {
            let data = queue.shift();
            data.rev = rev;
            socket.send(JSON.stringify(data));
        } else {
            queue.ready = true;
        }
    }

    function sendInsert(text_pos, ins) {
        let pos = countUtf8Bytes(text.substr(0, text_pos));
        queueSend({pos, rev, action: {Insert: ins}});
    }

    function sendDelete(text_pos, len) {
        let pos = countUtf8Bytes(text.substr(0, text_pos));
        queueSend({pos, rev, action: {Delete: len}});
    }

    function applyEdit(edit) {
        let unicode = toUTF8(text);
        let selStart = editor.selectionStart, selEnd = editor.selectionEnd;
        let preText = fromUTF8(unicode.substr(0, edit.pos));
        let oldI, newI;
        if (edit.action.Insert !== undefined) {
            oldI = edit.pos;
            newI = edit.pos + countUtf8Bytes(edit.action.Insert);
            text = preText + fromUTF8(toUTF8(edit.action.Insert) + unicode.substr(edit.pos));
            if (preText.length < selStart) {
                selStart += edit.action.Insert.length;
            }
            if (preText.length < selEnd) {
                selEnd += edit.action.Insert.length;
            }
        } else {
            oldI = edit.pos + edit.action.Delete;
            newI = edit.pos;
            text = preText + fromUTF8(unicode.substr(edit.pos + edit.action.Delete));
            if (preText.length + edit.action.Delete < selStart) {
                selStart -= edit.action.Delete;
            } else if (preText.length < selStart) {
                selStart = preText.length;
            }
            if (preText.length + edit.action.Delete < selEnd) {
                selEnd -= edit.action.Delete;
            } else if (preText.length < selEnd) {
                selEnd = preText.length;
            }
        }
        editor.value = text;
        editor.selectionStart = selStart;
        editor.selectionEnd = selEnd;
        rev = edit.rev;
        queue.forEach(transformEdit.bind(oldI, newI));
        queueReady();
    }

    let lastEvent = null;

    editor.addEventListener("keydown", function (event) {
        lastEvent = event;
    });

    editor.addEventListener('input', function () {
        if (editor.value === text) return;
        if (lastEvent) {
            let cursor = editor.selectionStart;
            // we don't care about control keys. Hack: only fast-path keys with a single letter
            if (lastEvent.key.length === 1) {
                // fast path for single letters
                let modifiedText = text.substr(0, cursor - 1) + lastEvent.key + text.substr(cursor);
                if (modifiedText === editor.value) {
                    sendInsert(cursor - 1, lastEvent.key);
                    text = modifiedText;
                    return;
                }
            } else if (lastEvent.key === "Backspace" || lastEvent.key === "Delete") {
                let modifiedText = text.substr(0, cursor) + text.substr(cursor + 1);
                if (modifiedText === editor.value) {
                    let deleted = text.substr(cursor, 1);
                    sendDelete(cursor, countUtf8Bytes(deleted));
                    text = modifiedText;
                    return;
                }
            }
        }
        // slow path, takes quadratic time w.r.t. diff range
        let diff = LCS(text, editor.value);
        for (const [t, idx, s] of diff) {
            if (t === INSERT) sendInsert(idx, s);
            if (t === DELETE) sendDelete(idx, countUtf8Bytes(s));
        }

        text = editor.value;
    });

    function LCS(a, b) {
        let aLen = a.length, bLen = b.length, i, j;
        let minLen = Math.min(aLen, bLen);
        let start = 0, tail = 0;
        while (start < minLen && a.charCodeAt(start) === b.charCodeAt(start)) start++;
        while (tail < (minLen - start) && a.charCodeAt(aLen - 1 - tail) === b.charCodeAt(bLen - 1 - tail)) tail++;
        aLen -= start + tail;
        bLen -= start + tail;

        let table = Array(aLen + 1);
        for (i = 0; i <= aLen; i++) table[i] = Array(bLen + 1).fill(0);
        for (i = 0; i < aLen; i++) {
            for (j = 0; j < bLen; j++) {
                table[i + 1][j + 1] = a.charCodeAt(start + i) === b.charCodeAt(start + j) ?
                    table[i][j] + 1 :
                    Math.max(table[i][j + 1], table[i + 1][j]);
            }
        }

        let diff = [];
        i = aLen;
        j = bLen;
        while (true) {
            if (j > 0 && (i === 0 || table[i][j] === table[i][j - 1])) {
                j--;
                diff.push([INSERT, j, b.charAt(start + j)]);
            } else if (i > 0 && (j === 0 || table[i][j] === table[i - 1][j])) {
                --i;
                diff.push([DELETE, i, a.charAt(start + i)]);
            } else if (i > 0 && j > 0) {
                i--;
                j--;
            } else {
                break;
            }
        }
        if (diff.length === 0) return [];

        let groupedDiff = [];
        let [currentType, currentIndex, currentString] = diff.pop();
        while (diff.length > 0) {
            let [t, ix, s] = diff.pop();
            if (t === currentType) {
                currentString += s;
            } else {
                groupedDiff.push([currentType, start + currentIndex, currentString]);
                currentType = t;
                currentString = s;
                currentIndex = ix;
            }
        }
        groupedDiff.push([currentType, start + currentIndex, currentString]);
        return groupedDiff;
    }
})();
